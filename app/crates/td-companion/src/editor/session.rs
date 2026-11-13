#[path = "templates.rs"]
mod templates;
pub use templates::NewLayout;
#[path = "sending.rs"]
mod sending;
pub use sending::{PendingSend, SaveState, SendReceipt};
#[path = "gifs.rs"]
mod gifs;
pub use gifs::GifProblem as Notice;
pub(crate) use gifs::GifSource;
pub use gifs::{GifEdit, GifPreview, GifProblem, GifWork};
#[path = "streams.rs"]
mod streams;
pub use streams::StreamChoice;
#[path = "placement.rs"]
mod placement;
pub use placement::{AddKind, AddTarget};
// Device-coupled editor facade. Keep one owner per Device; render its draft and selection.
use super::{Decision, Draft, Editor, EditorError};
use crate::{companion::Handle, layouts::LayoutError};
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
    #[error(transparent)]
    Asset(#[from] crate::features::assets::AssetError),
}

/// Blocking: `edit`, `resolve`, `begin_send` and the other changes write the drafts file before
/// they return. Run it on the editor [`Worker`](super::Worker)'s thread, never on a UI thread.
pub struct Session {
    handle: Handle,
    device: String,
    model: Editor,
    orientation: Orientation,
    page: Option<String>,
    tile: Option<String>,
    last_job: Option<crate::jobs::Job>,
    send_owner: std::sync::Arc<()>,
    sending: Option<u64>,
    send_serial: u64,
    observation_clock: std::sync::Arc<std::sync::atomic::AtomicU64>,
    observed: [u64; 2],
    gifs: gifs::GifState,
    changes: tokio::sync::broadcast::Receiver<crate::events::Change>,
    replacements: std::collections::BTreeMap<String, String>,
}
impl Session {
    pub(crate) fn open(
        handle: Handle,
        root: std::path::PathBuf,
        device: &str,
    ) -> Result<Self, EditorError> {
        let model = Editor::open(root, device)?;
        let changes = handle.observe();
        let mut session = Self {
            handle,
            device: device.into(),
            model,
            orientation: Orientation::Landscape,
            page: None,
            tile: None,
            last_job: None,
            send_owner: std::sync::Arc::new(()),
            sending: None,
            send_serial: 0,
            observation_clock: Default::default(),
            observed: [0; 2],
            gifs: Default::default(),
            changes,
            replacements: Default::default(),
        };
        session.refresh_assets()?;
        session.sync_gifs()?;
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
        self.cancel_replaced_gifs(orientation);
        self.refresh_assets()?;
        self.select_orientation(orientation);
        self.sync_gifs()?;
        self.model.persist()
    }
    pub fn edit(&mut self, edit: Edit) -> Result<Option<String>, EditorError> {
        self.refresh_assets()?;
        let added_page = match &edit {
            Edit::AddPage { page } => Some(page.id().to_owned()),
            _ => None,
        };
        let removes_page = matches!(&edit, Edit::RemovePage { .. });
        let result = self.model.edit(self.orientation, edit);
        self.refresh_assets()?;
        self.sync_gifs()?;
        self.reconcile_selection();
        if result.is_ok() {
            if let Some(page) = added_page {
                self.select(&page, None);
            } else if removes_page {
                self.tile = None;
            } else if let Ok(Some(id)) = &result {
                self.tile = Some(id.clone());
            }
        }
        self.model.persist()?;
        result
    }
    /// Drops this orientation's refused Label and Action values; admitted values stay.
    pub fn discard_rejected(&mut self) -> Result<(), EditorError> {
        self.model.discard_rejected(self.orientation)?;
        self.model.persist()
    }
    pub fn resolve(&mut self, decision: Decision) -> Result<(), EditorError> {
        let replacing = matches!(decision, Decision::UseDevice)
            && self.draft().is_some_and(|draft| draft.conflict().is_some());
        self.model.resolve(self.orientation, decision)?;
        if replacing {
            self.cancel_replaced_gifs(self.orientation);
            self.page = None;
            self.tile = None;
        }
        self.refresh_assets()?;
        self.reconcile_selection();
        self.sync_gifs()?;
        self.model.persist()
    }
    pub fn replace_assets(
        &mut self,
        replacements: std::collections::BTreeMap<String, String>,
    ) -> Result<(), EditorError> {
        self.model.replace_assets(replacements.clone())?;
        self.replacements.extend(replacements);
        self.model.persist()
    }
    /// Await a Companion change, updating drafts before the UI renders the next snapshot.
    /// Lagged receivers reread durable replacement state rather than losing notifications.
    pub async fn changed(&mut self) -> Result<crate::companion::Change, SessionError> {
        let change = match self.changes.recv().await {
            Ok(change) => change,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                crate::events::Change::AssetsReplaced
            }
            Err(_) => return Err(SessionError::Send("Companion stopped".into())),
        };
        self.refresh_assets()?;
        self.sync_gifs()?;
        if let crate::events::Change::JobUpdated(update) = &change
            && update.0.scope == crate::jobs::JobScope::Device(self.device.clone())
        {
            self.last_job = Some(update.0.clone());
        }
        Ok(change)
    }
    fn refresh_assets(&mut self) -> Result<(), EditorError> {
        let mut replacements = self.replacements.clone();
        replacements.extend(self.handle.brandfetch.replacements());
        if replacements.is_empty() {
            return Ok(());
        }
        let before = self.model.drafts.clone();
        self.model.replace_assets(replacements)?;
        if self.model.drafts != before {
            self.model.persist()?;
        }
        Ok(())
    }
    /// The draft owns the keep-set, including both orientations and legacy file icons.
    pub async fn prune_assets(
        &mut self,
    ) -> Result<crate::features::assets::PruneResult, SessionError> {
        self.refresh_assets()?;
        let mut keep = std::collections::BTreeSet::new();
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            if let Some(draft) = self.model.draft(orientation) {
                keep.extend(td_host::icons::file_ids(draft.layout()));
                keep.extend(td_host::icons::asset_ids(draft.layout()));
            }
        }
        let mut changes = self.handle.observe();
        let job = crate::features::assets::start_prune(
            &self.handle,
            self.device.clone(),
            keep.into_iter().collect(),
        )?;
        loop {
            let latest = self
                .handle
                .jobs()
                .get(&job.id)
                .ok_or_else(|| SessionError::Send("Job expired".into()))?;
            self.last_job = Some(latest.clone());
            if latest.state.finished() {
                if let Some(error) = latest.error {
                    return Err(SessionError::Job(error));
                }
                return serde_json::from_value(latest.result.unwrap_or_default())
                    .map_err(|e| SessionError::Send(e.to_string()));
            }
            match changes.recv().await {
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return Err(SessionError::Send("Companion stopped".into())),
            }
        }
    }
    pub async fn load(&mut self) -> Result<(), SessionError> {
        // Numbered at issue, like a send's readback: see `sending::execute`.
        let observation = self.next_observation();
        let read = match self
            .handle
            .layout_operations()
            .read(self.device.clone(), self.orientation)
            .await
        {
            Ok(read) => read,
            Err(error @ LayoutError::NotFound) => {
                self.observed[super::index(self.orientation)] = observation;
                if self.draft().is_some() {
                    self.model.receive_missing(self.orientation)?;
                    self.model.persist()?;
                }
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        };
        self.observed[super::index(self.orientation)] = observation;
        self.model.receive(read.layout, read.revision)?;
        if self.draft().is_some_and(|draft| draft.conflict().is_none()) {
            self.page = None;
            self.tile = None;
        }
        self.refresh_assets()?;
        self.sync_gifs()?;
        self.reconcile_selection();
        self.model.persist()?;
        Ok(())
    }
    /// Convenience for callers that do not need to edit during the transfer.
    pub async fn send(&mut self, capabilities: Capabilities<'_>) -> Result<(), SessionError> {
        let pending = self.begin_send(capabilities)?;
        self.finish_send(pending.wait().await)
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
            Self::Asset(error) => Some(error.body()),
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

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel_gifs();
    }
}
