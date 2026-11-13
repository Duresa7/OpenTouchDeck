#[path = "placement.rs"]
mod placement;
pub use placement::{AddKind, AddTarget};
// Device-coupled editor facade. Keep one owner per Device; render its draft and selection.
use super::{Decision, Draft, Editor, EditorError};
use crate::{
    companion::Handle,
    jobs::JobState,
    layouts::{IfMatch, LayoutError},
};
use td_core::{
    Layout, Orientation,
    editing::{Capabilities, Edit},
};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("{file} is not a Layout file. Nothing changed: {reason}")]
    Import {
        file: String,
        reason: td_core::editing::EditError,
    },
    #[error(transparent)]
    Editor(#[from] EditorError),
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error("Layout send did not succeed: {0}")]
    Send(String),
    #[error("{0:?}")]
    Job(crate::api::ErrorBody),
}

pub struct Session {
    handle: Handle,
    device: String,
    model: Editor,
    orientation: Orientation,
    page: Option<String>,
    tile: Option<String>,
    last_job: Option<crate::jobs::Job>,
}
impl Session {
    pub(crate) fn open(
        handle: Handle,
        root: std::path::PathBuf,
        device: &str,
    ) -> Result<Self, EditorError> {
        let model = Editor::open(root, device)?;
        let mut session = Self {
            handle,
            device: device.into(),
            model,
            orientation: Orientation::Landscape,
            page: None,
            tile: None,
            last_job: None,
        };
        session.reconcile_selection();
        Ok(session)
    }
    /// The same typed change stream used by the Companion's native adapters.
    pub fn observe(&self) -> tokio::sync::broadcast::Receiver<crate::events::Change> {
        self.handle.observe()
    }
    pub fn last_job(&self) -> Option<&crate::jobs::Job> {
        self.last_job.as_ref()
    }
    pub fn draft(&self) -> Option<&Draft> {
        self.model.draft(self.orientation)
    }
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }
    pub fn selection(&self) -> (Option<&str>, Option<&str>) {
        (self.page.as_deref(), self.tile.as_deref())
    }
    pub fn select_orientation(&mut self, orientation: Orientation) {
        self.orientation = orientation;
        self.page = None;
        self.tile = None;
        self.reconcile_selection();
    }
    /// Invalid selections are ignored; a Page selection enters Page toolbar mode.
    pub fn select(&mut self, page: &str, tile: Option<&str>) {
        let Some(draft) = self.draft() else { return };
        let Some(found) = draft.layout().pages.iter().find(|p| p.id() == page) else {
            return;
        };
        if let Some(id) = tile
            && !matches!(found, td_core::layout::Page::Grid { tiles, .. } if tiles.iter().any(|t| t.id == id))
        {
            return;
        }
        self.page = Some(page.into());
        self.tile = tile.map(str::to_owned);
    }
    /// Parse before touching either orientation, selection or persisted state.
    pub fn import(&mut self, file: &str, bytes: &[u8]) -> Result<(), SessionError> {
        let document =
            td_core::editing::Document::parse(bytes).map_err(|reason| SessionError::Import {
                file: file.into(),
                reason,
            })?;
        self.start(document.layout().clone())?;
        Ok(())
    }
    pub fn export(&self) -> Result<String, EditorError> {
        let draft = self.draft().ok_or(EditorError::NoDraft)?;
        Ok(String::from_utf8(draft.layout().to_canonical_json()).expect("Layout JSON is UTF-8"))
    }
    pub fn start(&mut self, layout: Layout) -> Result<(), EditorError> {
        let orientation = layout.orientation;
        self.model.start(layout)?;
        self.select_orientation(orientation);
        self.model.persist()
    }
    pub fn edit(&mut self, edit: Edit) -> Result<Option<String>, EditorError> {
        let result = self.model.edit(self.orientation, edit);
        self.reconcile_selection();
        if let Ok(Some(id)) = &result {
            self.tile = Some(id.clone());
        }
        self.model.persist()?;
        result
    }
    pub fn resolve(&mut self, decision: Decision) -> Result<(), EditorError> {
        self.model.resolve(self.orientation, decision)?;
        self.reconcile_selection();
        self.model.persist()
    }
    pub fn replace_assets(
        &mut self,
        replacements: std::collections::BTreeMap<String, String>,
    ) -> Result<(), EditorError> {
        self.model.replace_assets(replacements)?;
        self.model.persist()
    }
    pub async fn load(&mut self) -> Result<(), SessionError> {
        let read = match self
            .handle
            .layout_operations()
            .read(self.device.clone(), self.orientation)
            .await
        {
            Ok(read) => read,
            Err(error @ LayoutError::NotFound) => {
                if self.draft().is_some() {
                    self.model.receive_missing(self.orientation)?;
                    self.model.persist()?;
                }
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        };
        self.model.receive(read.layout, read.revision)?;
        self.reconcile_selection();
        self.model.persist()?;
        Ok(())
    }
    /// Runs the conditional send and checks the Device readback before marking the draft sent.
    /// A conflict is captured for resolve(); the caller can render draft().conflict().
    pub async fn send(&mut self, capabilities: Capabilities<'_>) -> Result<(), SessionError> {
        let plan = match self.model.prepare_push(self.orientation, capabilities) {
            Ok(plan) => plan,
            Err(error) => {
                self.select_first_problem();
                return Err(error.into());
            }
        };
        let operations = self.handle.layout_operations();
        let mut changes = self.handle.observe();
        let pushed = operations
            .push(
                self.device.clone(),
                self.orientation,
                plan.layout().to_canonical_json(),
                plan.expected_revision().map(|r| IfMatch(vec![r.into()])),
            )
            .await;
        let job = match pushed {
            Ok(job) => job,
            Err(error @ LayoutError::Changed { .. }) => {
                match operations.read(self.device.clone(), self.orientation).await {
                    Ok(read) => self.model.receive(read.layout, read.revision)?,
                    Err(LayoutError::NotFound) => self.model.receive_missing(self.orientation)?,
                    Err(error) => return Err(error.into()),
                }
                self.model.persist()?;
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        };
        loop {
            let latest = self
                .handle
                .jobs()
                .get(&job.id)
                .ok_or_else(|| SessionError::Send("Job expired".into()))?;
            self.last_job = Some(latest.clone());
            if latest.state.finished() {
                if latest.state != JobState::Succeeded {
                    if latest
                        .error
                        .as_ref()
                        .is_some_and(|e| e.code == "layout.changed")
                    {
                        match operations.read(self.device.clone(), self.orientation).await {
                            Ok(read) => self.model.receive(read.layout, read.revision)?,
                            Err(LayoutError::NotFound) => {
                                self.model.receive_missing(self.orientation)?
                            }
                            Err(error) => return Err(error.into()),
                        }
                        self.model.persist()?;
                    }
                    return Err(latest.error.map(SessionError::Job).unwrap_or_else(|| {
                        SessionError::Send("Sending stopped. The Device keeps its Layout.".into())
                    }));
                }
                break;
            }
            match changes.recv().await {
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return Err(SessionError::Send("Companion stopped".into())),
            }
        }
        let read = operations
            .read(self.device.clone(), self.orientation)
            .await?;
        self.model.finish_push(plan, read.layout, read.revision)?;
        self.model.persist()?;
        Ok(())
    }
    fn select_first_problem(&mut self) {
        let target = self.draft().and_then(|d| d.layout().pages.iter().find_map(|p| match p {
            td_core::layout::Page::Grid { id, tiles, .. } => tiles.iter().find(|t| d.rejected().iter().any(|r| matches!(&r.edit, Edit::Label { tile, .. } | Edit::Action { tile, .. } if tile == &t.id))).map(|t| (id.clone(), t.id.clone())),
            _ => None,
        }));
        if let Some((page, tile)) = target {
            self.page = Some(page);
            self.tile = Some(tile);
        }
    }
    fn reconcile_selection(&mut self) {
        let layout = self.model.draft(self.orientation).map(|d| d.layout());
        if !layout.is_some_and(|l| l.pages.iter().any(|p| Some(p.id()) == self.page.as_deref())) {
            self.page = layout.and_then(|l| l.pages.first()).map(|p| p.id().into());
            self.tile = None;
        }
        if !layout.is_some_and(|l| l.pages.iter().any(|p| matches!(p, td_core::layout::Page::Grid { id, tiles, .. } if Some(id.as_str()) == self.page.as_deref() && tiles.iter().any(|t| Some(t.id.as_str()) == self.tile.as_deref())))) { self.tile = None; }
    }
}

impl SessionError {
    /// Preserve machine-readable refusals (including 412/422 and Device/Job failures).
    pub fn refusal(&self) -> Option<crate::api::ErrorBody> {
        use crate::api::ApiError;
        match self {
            Self::Layout(error) => Some(error.body()),
            Self::Job(error) => Some(error.clone()),
            Self::Editor(EditorError::Edit(td_core::editing::EditError::Invalid(problems))) => {
                Some(crate::api::ErrorBody {
                    code: "layout.invalid".into(),
                    message: "The Layout has problems.".into(),
                    details: Some(serde_json::json!({"issues": problems})),
                })
            }
            _ => None,
        }
    }
}
