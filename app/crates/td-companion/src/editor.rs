//! Headless Layout editing. Session couples a persisted Editor to the Companion Device lane.
//! Editor remains usable independently for deterministic lifecycle tests and offline drafts.
//! Keep one owner per Device; UI adapters render snapshots and own dialogs and window state.
mod presets;
pub use presets::placed_appearance;
mod session;
use serde::{Deserialize, Serialize};
pub(crate) use session::GifSource;
pub use session::{
    AddKind, AddTarget, GifEdit, GifPreview, GifProblem, GifWork, NewLayout, Notice, PendingSend,
    SaveState, SendReceipt, Session, SessionError, StreamChoice,
};
use std::{collections::BTreeMap, io, path::PathBuf};
use td_core::{
    Layout, Orientation,
    editing::{Capabilities, Compatibility, Document, Edit, EditError},
    layout::{Page, Tile},
};

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("invalid Device id")]
    InvalidDevice,
    #[error("no draft for this orientation")]
    NoDraft,
    #[error("a Layout send is already running")]
    Sending,
    #[error("Device work requires a running Tokio runtime")]
    RuntimeUnavailable,
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("unresolved rejected fields")]
    Rejected,
    #[error("resolve the Device Layout conflict first")]
    Conflict,
    #[error("Device capabilities are unknown")]
    UnknownCapabilities,
    #[error("Layout is incompatible: {0:?}")]
    Incompatible(Vec<td_core::editing::Problem>),
    #[error("the Device did not show the sent Layout")]
    PushMismatch,
    #[error("push belongs to another Device or its baseline changed")]
    StalePush,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    pub edit: Edit,
    pub reason: EditError,
}
impl Rejected {
    /// The form field a refused action names, independent of its message wording.
    pub fn action_field(&self) -> Option<&str> {
        let EditError::Invalid(problems) = &self.reason else {
            return None;
        };
        let path = &problems.first()?.path;
        let (_, suffix) = path.split_once("/action/")?;
        suffix.split('/').next().filter(|field| !field.is_empty())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCopy {
    document: Document,
    revision: String,
}
impl DeviceCopy {
    pub fn layout(&self) -> &Layout {
        self.document.layout()
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    document: Document,
    known: Option<DeviceCopy>,
    rejected: Vec<Rejected>,
    conflict: Option<DeviceCopy>,
    unconfirmed: bool,
}
impl Draft {
    pub fn layout(&self) -> &Layout {
        self.document.layout()
    }
    pub fn rejected(&self) -> &[Rejected] {
        &self.rejected
    }
    pub fn conflict(&self) -> Option<&DeviceCopy> {
        self.conflict.as_ref()
    }
    /// A write may have reached the Device without a confirming observation.
    pub fn is_unconfirmed(&self) -> bool {
        self.unconfirmed
    }
    pub fn is_sent(&self) -> bool {
        !self.unconfirmed
            && self.rejected.is_empty()
            && self.conflict.is_none()
            && self
                .known
                .as_ref()
                .is_some_and(|known| known.document == self.document)
    }
    /// One effective Tile value for every consumer, including values refused by admission.
    /// Diagnostics are available alongside it in rejected().
    pub fn effective_tile(&self, id: &str) -> Option<Tile> {
        let mut tile = self.layout().pages.iter().find_map(|page| match page {
            Page::Grid { tiles, .. } => tiles.iter().find(|tile| tile.id == id).cloned(),
            _ => None,
        })?;
        for refused in &self.rejected {
            match &refused.edit {
                Edit::Label {
                    tile: target,
                    label,
                } if target == id => tile.label = label.clone(),
                Edit::Action {
                    tile: target,
                    action,
                } if target == id => tile.action = action.clone(),
                _ => {}
            }
        }
        Some(tile)
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Decision {
    KeepMine,
    UseDevice,
}

/// An immutable send snapshot; edits made during a send remain unsent afterwards.
#[derive(Debug, Clone)]
pub struct PushPlan {
    device: String,
    document: Document,
    baseline: Option<DeviceCopy>,
    receive_serial: u64,
}
impl PushPlan {
    pub fn layout(&self) -> &Layout {
        self.document.layout()
    }
    pub fn expected_revision(&self) -> Option<&str> {
        self.baseline.as_ref().map(|copy| copy.revision.as_str())
    }
}

#[derive(Clone)]
pub struct Editor {
    path: PathBuf,
    device: String,
    drafts: [Option<Draft>; 2],
    receive_serial: [u64; 2],
}
impl Editor {
    /// Blocking disk read. Corrupt/unsupported storage is reported, never silently overwritten.
    /// root is the Companion's data directory; drafts are separate from stored and Pushed copies.
    pub fn open(root: PathBuf, device: &str) -> Result<Self, EditorError> {
        if device.is_empty()
            || !device
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(EditorError::InvalidDevice);
        }
        let path = root.join("editor-drafts-v1").join(format!("{device}.json"));
        let mut editor = Self {
            path,
            device: device.into(),
            drafts: [None, None],
            receive_serial: [0, 0],
        };
        match std::fs::read(&editor.path) {
            Ok(bytes) => {
                let stored: Stored = serde_json::from_slice(&bytes).map_err(bad_storage)?;
                if stored.version != 1 {
                    return Err(bad_storage("unsupported editor storage version").into());
                }
                for draft in stored.drafts {
                    let Ok(document) = Document::parse(draft.layout.as_bytes()) else {
                        continue;
                    };
                    let orientation = document.layout().orientation;
                    let slot = &mut editor.drafts[index(orientation)];
                    if slot.is_some() {
                        return Err(bad_storage("duplicate draft orientation").into());
                    }
                    let known = draft
                        .known
                        .map(|copy| copy.admit(orientation))
                        .transpose()?;
                    let conflict = draft
                        .conflict
                        .map(|copy| copy.admit(orientation))
                        .transpose()?;
                    let mut restored = Draft {
                        document,
                        known,
                        conflict,
                        unconfirmed: draft.unconfirmed,
                        rejected: Vec::new(),
                    };
                    // Recompute diagnostics through today's interface; don't trust saved error text.
                    for edit in draft.rejected {
                        if field(&edit).is_none() {
                            return Err(bad_storage("invalid rejected edit").into());
                        }
                        match restored.document.apply(&edit) {
                            Err(reason) => {
                                if field(&edit).is_some_and(|(tile, _)| {
                                    restored.effective_tile(tile).is_some()
                                }) {
                                    remember(&mut restored.rejected, Rejected { edit, reason });
                                }
                            }
                            Ok(change) => restored.document = change.document,
                        }
                    }
                    *slot = Some(restored);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(editor)
    }
    pub fn draft(&self, orientation: Orientation) -> Option<&Draft> {
        self.drafts[index(orientation)].as_ref()
    }
    fn draft_mut(&mut self, orientation: Orientation) -> Result<&mut Draft, EditorError> {
        self.drafts[index(orientation)]
            .as_mut()
            .ok_or(EditorError::NoDraft)
    }
    /// Start/import a draft, preserving its last known Device revision for conditional sending.
    pub fn start(&mut self, layout: Layout) -> Result<(), EditorError> {
        let document = Document::admit(layout)?;
        let slot = &mut self.drafts[index(document.layout().orientation)];
        let known = slot.as_ref().and_then(|draft| draft.known.clone());
        let conflict = slot.as_ref().and_then(|draft| draft.conflict.clone());
        let unconfirmed = slot.as_ref().is_some_and(|draft| draft.unconfirmed);
        *slot = Some(Draft {
            document,
            known,
            conflict,
            unconfirmed,
            rejected: Vec::new(),
        });
        Ok(())
    }
    /// Structural refusals leave the draft unchanged. Refused label/action values remain visible
    /// through effective_tile and block sending until corrected or explicitly discarded.
    pub fn edit(
        &mut self,
        orientation: Orientation,
        mut edit: Edit,
    ) -> Result<Option<String>, EditorError> {
        let draft = self.draft_mut(orientation)?;
        if let Edit::Sink { tile, sink } = &edit {
            let action = draft
                .effective_tile(tile)
                .ok_or_else(|| EditError::MissingTile(tile.clone()))?
                .action;
            edit = Edit::Action {
                tile: tile.clone(),
                action: td_core::editing::action_with_sink(action, sink.clone())?,
            };
        }
        match draft.document.apply(&edit) {
            Ok(change) => {
                let copied = if let Edit::Duplicate { tile } = &edit {
                    draft
                        .rejected
                        .iter()
                        .filter(|rejected| field(&rejected.edit).is_some_and(|(id, _)| id == tile))
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                draft.document = change.document;
                if let Some(key) = field(&edit) {
                    draft.rejected.retain(|r| field(&r.edit) != Some(key));
                }
                if let Edit::Renderer {
                    tile,
                    appearance:
                        td_core::editing::Appearance::Live { .. }
                        | td_core::editing::Appearance::Plain {
                            action: Some(_), ..
                        },
                } = &edit
                {
                    draft
                        .rejected
                        .retain(|r| field(&r.edit) != Some((tile.as_str(), "action")));
                }
                draft.rejected.retain(|r| {
                    field(&r.edit).is_some_and(|(tile, _)| has_tile(draft.document.layout(), tile))
                });
                if let Some(id) = &change.created_tile {
                    for mut rejected in copied {
                        match &mut rejected.edit {
                            Edit::Label { tile, .. } | Edit::Action { tile, .. } => {
                                *tile = id.clone()
                            }
                            _ => unreachable!(),
                        }
                        // Paths depend on the newly placed Tile's position in the Layout.
                        if let Err(reason) = draft.document.apply(&rejected.edit) {
                            rejected.reason = reason;
                            remember(&mut draft.rejected, rejected);
                        }
                    }
                }
                reconcile_rejected(draft);
                Ok(change.created_tile)
            }
            Err(reason) => {
                if field(&edit).is_some_and(|(tile, _)| has_tile(draft.layout(), tile)) {
                    remember(
                        &mut draft.rejected,
                        Rejected {
                            edit,
                            reason: reason.clone(),
                        },
                    );
                }
                Err(reason.into())
            }
        }
    }
    pub fn discard_rejected(&mut self, orientation: Orientation) -> Result<(), EditorError> {
        self.draft_mut(orientation)?.rejected.clear();
        Ok(())
    }
    /// Applies replacements transactionally across both orientations. Device copies stay unchanged.
    pub fn replace_assets(
        &mut self,
        replacements: BTreeMap<String, String>,
    ) -> Result<(), EditorError> {
        let mut drafts = self.drafts.clone();
        for draft in drafts.iter_mut().flatten() {
            draft.document = draft
                .document
                .apply(&Edit::ReplaceAssets {
                    replacements: replacements.clone(),
                })?
                .document;
            reconcile_rejected(draft);
        }
        self.drafts = drafts;
        Ok(())
    }
    /// A read after load or a conflicting send. A different draft requires an explicit decision.
    pub fn receive(&mut self, layout: Layout, revision: String) -> Result<(), EditorError> {
        let document = Document::admit(layout)?;
        let slot = &mut self.drafts[index(document.layout().orientation)];
        let remote = DeviceCopy { document, revision };
        self.receive_serial[index(remote.document.layout().orientation)] += 1;
        if let Some(draft) = slot {
            draft.unconfirmed = false;
        }
        match slot {
            Some(draft) if draft.document != remote.document || !draft.rejected.is_empty() => {
                draft.conflict = Some(remote)
            }
            Some(draft) => {
                draft.known = Some(remote);
                draft.conflict = None;
            }
            None => {
                *slot = Some(Draft {
                    document: remote.document.clone(),
                    known: Some(remote),
                    rejected: Vec::new(),
                    conflict: None,
                    unconfirmed: false,
                })
            }
        }
        Ok(())
    }
    /// A conditional-send read found no stored Layout. Keep the draft and retry without a
    /// stale revision, matching the editor's first-send policy.
    pub fn receive_missing(&mut self, orientation: Orientation) -> Result<(), EditorError> {
        let draft = self.draft_mut(orientation)?;
        draft.known = None;
        draft.conflict = None;
        draft.unconfirmed = false;
        Ok(())
    }
    pub fn resolve(
        &mut self,
        orientation: Orientation,
        decision: Decision,
    ) -> Result<(), EditorError> {
        let draft = self.draft_mut(orientation)?;
        if let Some(remote) = draft.conflict.take() {
            if matches!(decision, Decision::UseDevice) {
                draft.document = remote.document.clone();
                draft.rejected.clear();
            }
            draft.known = Some(remote);
            draft.unconfirmed = false;
        }
        Ok(())
    }
    fn mark_unconfirmed(&mut self, orientation: Orientation) -> Result<(), EditorError> {
        self.draft_mut(orientation)?.unconfirmed = true;
        Ok(())
    }
    pub fn prepare_push(
        &self,
        orientation: Orientation,
        capabilities: Capabilities<'_>,
    ) -> Result<PushPlan, EditorError> {
        let draft = self.draft(orientation).ok_or(EditorError::NoDraft)?;
        if !draft.rejected.is_empty() {
            return Err(EditorError::Rejected);
        }
        if draft.conflict.is_some() {
            return Err(EditorError::Conflict);
        }
        match draft.document.compatibility(capabilities) {
            Compatibility::Supported => Ok(PushPlan {
                device: self.device.clone(),
                document: draft.document.clone(),
                baseline: draft.known.clone(),
                receive_serial: self.receive_serial[index(orientation)],
            }),
            Compatibility::Unknown => Err(EditorError::UnknownCapabilities),
            Compatibility::Unsupported(problems) => Err(EditorError::Incompatible(problems)),
        }
    }
    /// Only a matching readback establishes a sent copy. Newer edits are kept, still unsent.
    pub fn finish_push(
        &mut self,
        plan: PushPlan,
        displayed: Layout,
        revision: String,
    ) -> Result<(), EditorError> {
        if plan.device != self.device {
            return Err(EditorError::StalePush);
        }
        let received_since =
            self.receive_serial[index(plan.layout().orientation)] != plan.receive_serial;
        let draft = self.draft_mut(plan.layout().orientation)?;
        let identical_receive = received_since
            && draft
                .known
                .as_ref()
                .is_some_and(|known| known.document == plan.document)
            && draft.known != plan.baseline;
        if (draft.known != plan.baseline && !identical_receive) || draft.conflict.is_some() {
            return Err(EditorError::StalePush);
        }
        if Document::admit(displayed)? != plan.document {
            return Err(EditorError::PushMismatch);
        }
        draft.unconfirmed = false;
        if identical_receive {
            return Ok(());
        }
        draft.known = Some(DeviceCopy {
            document: plan.document,
            revision,
        });
        Ok(())
    }
    /// Blocking, explicit checkpoint. One atomic file holds both orientations and their decisions.
    pub fn persist(&self) -> Result<(), EditorError> {
        let stored = Stored {
            version: 1,
            drafts: self
                .drafts
                .iter()
                .flatten()
                .map(|draft| StoredDraft {
                    layout: text(&draft.document),
                    unconfirmed: draft.unconfirmed,
                    known: draft.known.as_ref().map(StoredCopy::from),
                    conflict: draft.conflict.as_ref().map(StoredCopy::from),
                    rejected: draft.rejected.iter().map(|r| r.edit.clone()).collect(),
                })
                .collect(),
        };
        td_core::app_dirs::write_atomic(
            &self.path,
            &serde_json::to_vec(&stored).map_err(bad_storage)?,
        )?;
        Ok(())
    }
}
fn index(orientation: Orientation) -> usize {
    match orientation {
        Orientation::Landscape => 0,
        Orientation::Portrait => 1,
    }
}
fn field(edit: &Edit) -> Option<(&str, &str)> {
    match edit {
        Edit::Label { tile, .. } => Some((tile, "label")),
        Edit::Action { tile, .. } => Some((tile, "action")),
        _ => None,
    }
}
// Recheck paths after structural edits. A field that becomes admissible (for example after
// adding its navigation target) joins the draft; otherwise its effective value stays rejected.
fn reconcile_rejected(draft: &mut Draft) {
    for mut rejected in std::mem::take(&mut draft.rejected) {
        if !field(&rejected.edit).is_some_and(|(tile, _)| has_tile(draft.layout(), tile)) {
            continue;
        }
        match draft.document.apply(&rejected.edit) {
            Ok(change) => draft.document = change.document,
            Err(reason) => {
                rejected.reason = reason;
                draft.rejected.push(rejected);
            }
        }
    }
}
fn remember(rejected: &mut Vec<Rejected>, next: Rejected) {
    rejected.retain(|old| field(&old.edit) != field(&next.edit));
    rejected.push(next);
}
fn has_tile(layout: &Layout, id: &str) -> bool {
    layout.pages.iter().any(
        |page| matches!(page, Page::Grid { tiles, .. } if tiles.iter().any(|tile| tile.id == id)),
    )
}
fn text(document: &Document) -> String {
    String::from_utf8(document.layout().to_canonical_json()).expect("Layout JSON is UTF-8")
}
fn bad_storage(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
#[derive(Serialize, Deserialize)]
struct Stored {
    version: u8,
    drafts: Vec<StoredDraft>,
}
#[derive(Serialize, Deserialize)]
struct StoredDraft {
    layout: String,
    #[serde(default)]
    unconfirmed: bool,
    known: Option<StoredCopy>,
    conflict: Option<StoredCopy>,
    rejected: Vec<Edit>,
}
#[derive(Serialize, Deserialize)]
struct StoredCopy {
    layout: String,
    revision: String,
}
impl From<&DeviceCopy> for StoredCopy {
    fn from(copy: &DeviceCopy) -> Self {
        Self {
            layout: text(&copy.document),
            revision: copy.revision.clone(),
        }
    }
}
impl StoredCopy {
    fn admit(self, orientation: Orientation) -> Result<DeviceCopy, EditorError> {
        let document = Document::parse(self.layout.as_bytes())?;
        if document.layout().orientation != orientation {
            return Err(bad_storage("Device copy orientation differs from draft").into());
        }
        Ok(DeviceCopy {
            document,
            revision: self.revision,
        })
    }
}
