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
    #[error(transparent)]
    Editor(#[from] EditorError),
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error("Layout send did not succeed: {0}")]
    Send(String),
}

pub struct Session {
    handle: Handle,
    device: String,
    model: Editor,
    orientation: Orientation,
    page: Option<String>,
    tile: Option<String>,
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
        };
        session.reconcile_selection();
        Ok(session)
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
        let read = self
            .handle
            .layout_operations()
            .read(self.device.clone(), self.orientation)
            .await?;
        self.model.receive(read.layout, read.revision)?;
        self.reconcile_selection();
        self.model.persist()?;
        Ok(())
    }
    /// Runs the conditional send and checks the Device readback before marking the draft sent.
    /// A conflict is captured for resolve(); the caller can render draft().conflict().
    pub async fn send(&mut self, capabilities: Capabilities<'_>) -> Result<(), SessionError> {
        let plan = self.model.prepare_push(self.orientation, capabilities)?;
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
                    return Err(SessionError::Send(format!("{:?}", latest.error)));
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
    fn reconcile_selection(&mut self) {
        let layout = self.model.draft(self.orientation).map(|d| d.layout());
        if !layout.is_some_and(|l| l.pages.iter().any(|p| Some(p.id()) == self.page.as_deref())) {
            self.page = layout.and_then(|l| l.pages.first()).map(|p| p.id().into());
            self.tile = None;
        }
        if !layout.is_some_and(|l| l.pages.iter().any(|p| matches!(p, td_core::layout::Page::Grid { id, tiles, .. } if Some(id.as_str()) == self.page.as_deref() && tiles.iter().any(|t| Some(t.id.as_str()) == self.tile.as_deref())))) { self.tile = None; }
    }
}
