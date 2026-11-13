//! One Device's editor as a window drives it. The Worker owns the [`Session`] on one dedicated
//! thread, so its disk writes and Device reads never run on a caller's (UI) thread. Commands go
//! in through [`Worker::dispatch`], in order; every change comes out as an [`EditorView`] on a
//! `watch` channel. Dropping the last `Worker` clone ends the thread, and the Session's `Drop`
//! cancels its GIF work.
use super::{
    AddKind, AddTarget, Decision, EditorError, GifEdit, GifWork, NewLayout, Notice, PendingSend,
    Rejected, SaveState, SendReceipt, Session, SessionError,
};
use crate::{
    api::ErrorBody,
    companion::Handle,
    devices::{ControlLayout, DeviceId},
    features::assets::Fit,
    layouts::{LayoutError, PushProgress},
};
use std::{future::Future, pin::Pin, sync::Arc};
use td_core::{
    Orientation,
    editing::{Capabilities, Edit},
    layout::{Page, Tile},
};
use tokio::sync::{mpsc, watch};

/// One editor control's request. Each maps to one Session call.
pub enum Command {
    /// A filmstrip entry, a Tile, or Done (no Tile).
    Select {
        page: String,
        tile: Option<String>,
    },
    /// Shows the draft for `orientation`, then reads the Device's Layout for it.
    Orientation(Orientation),
    Edit(Edit),
    /// Drops the draft's refused Label and Action values.
    DiscardRejected,
    Add {
        kind: AddKind,
        target: AddTarget,
    },
    AddGif {
        file: String,
        bytes: Vec<u8>,
        target: AddTarget,
    },
    Gif {
        tile: String,
        edit: GifEdit,
    },
    Resolve(Decision),
    /// Sends the draft with the Device's own capabilities. Its outcome lands with the receipt.
    Send,
    NewLayout(NewLayout),
    Import {
        file: String,
        bytes: Vec<u8>,
    },
    Export,
    Load,
}
/// Which command an [`Outcome`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Select,
    Orientation,
    Edit,
    DiscardRejected,
    Add,
    AddGif,
    Gif,
    Resolve,
    Send,
    NewLayout,
    Import,
    Export,
    Load,
}
impl Command {
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::Select { .. } => CommandKind::Select,
            Self::Orientation(_) => CommandKind::Orientation,
            Self::Edit(_) => CommandKind::Edit,
            Self::DiscardRejected => CommandKind::DiscardRejected,
            Self::Add { .. } => CommandKind::Add,
            Self::AddGif { .. } => CommandKind::AddGif,
            Self::Gif { .. } => CommandKind::Gif,
            Self::Resolve(_) => CommandKind::Resolve,
            Self::Send => CommandKind::Send,
            Self::NewLayout(_) => CommandKind::NewLayout,
            Self::Import { .. } => CommandKind::Import,
            Self::Export => CommandKind::Export,
            Self::Load => CommandKind::Load,
        }
    }
}

/// The result of the last finished command. `number` grows with every outcome, so two equal
/// failures in a row are still two outcomes.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub number: u64,
    pub command: CommandKind,
    pub result: Result<Done, Failure>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    Changed,
    /// The Tile an edit or Add Tile created.
    Tile(String),
    /// The draft's Layout JSON, for the window to save.
    Export(String),
}
/// A refused command: the copy to show and the typed error behind it.
#[derive(Debug, Clone)]
pub struct Failure {
    pub notice: Notice,
    pub error: Arc<SessionError>,
}
/// Two failures are the same only when they are the same report, never because the text matches.
impl PartialEq for Failure {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.error, &other.error)
    }
}
impl Failure {
    fn new(error: SessionError, notice: Notice) -> Self {
        Self {
            notice,
            error: Arc::new(error),
        }
    }
    fn plain(error: SessionError) -> Self {
        let notice = Notice {
            text: error.to_string(),
            detail: String::new(),
        };
        Self::new(error, notice)
    }
    /// The machine-readable refusal, when there is one.
    pub fn refusal(&self) -> Option<ErrorBody> {
        self.error.refusal()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenState {
    Opening,
    Ready,
    /// The saved drafts couldn't be read; nothing was overwritten.
    Failed(Failure),
}
/// Everything the editor screens show, as plain data.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorView {
    pub open: OpenState,
    pub device: String,
    pub device_name: String,
    pub orientation: Orientation,
    pub save_state: SaveState,
    /// The draft for `orientation`; `None` before one is started, imported or read.
    pub layout: Option<LayoutView>,
    pub page: Option<String>,
    pub tile: Option<String>,
    pub conflict: Option<ConflictView>,
    pub add_target: Option<AddTarget>,
    pub push_progress: Option<PushProgress>,
    pub offers_gif: bool,
    pub control_layout: ControlLayout,
    pub outcome: Option<Outcome>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutView {
    pub name: String,
    pub full_screen: bool,
    pub pages: Vec<PageView>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PageView {
    pub id: String,
    pub title: String,
    /// A Usage Page (a dashboard); it has no grid or Tiles.
    pub usage: bool,
    pub rows: u8,
    pub cols: u8,
    pub tiles: Vec<TileView>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TileView {
    /// The effective Tile, including refused Label and Action values.
    pub tile: Tile,
    /// Its refused fields; `Rejected::action_field` names the Action field at fault.
    pub problems: Vec<Rejected>,
    pub gif: Option<GifWork>,
    pub gif_fit: Fit,
}
/// The Device holds a different Layout than the draft; the person decides which stays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictView {
    /// The Layout name on the Device.
    pub name: String,
}
impl EditorView {
    fn opening(device: &str) -> Self {
        Self {
            open: OpenState::Opening,
            device: device.into(),
            device_name: String::new(),
            orientation: Orientation::Landscape,
            save_state: SaveState::None,
            layout: None,
            page: None,
            tile: None,
            conflict: None,
            add_target: None,
            push_progress: None,
            offers_gif: false,
            control_layout: ControlLayout::default(),
            outcome: None,
        }
    }
}

struct Inner {
    commands: mpsc::UnboundedSender<Command>,
    view: watch::Receiver<EditorView>,
}
/// A window's handle to one Device's editor. Clone it freely; the last clone's drop ends it.
#[derive(Clone)]
pub struct Worker {
    inner: Arc<Inner>,
}
impl Worker {
    /// Starts the thread; the Session opens on it. Needs a Tokio runtime context.
    pub(crate) fn open(handle: Handle, device: &str) -> Result<Self, EditorError> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| EditorError::RuntimeUnavailable)?;
        let (commands, queue) = mpsc::unbounded_channel();
        let (publish, view) = watch::channel(EditorView::opening(device));
        let device = device.to_owned();
        std::thread::Builder::new()
            .name("editor-worker".into())
            .spawn(move || runtime.block_on(run(handle, device, queue, publish)))?;
        Ok(Self {
            inner: Arc::new(Inner { commands, view }),
        })
    }
    /// Queues a command. Commands run in the order they were dispatched; never blocks.
    pub fn dispatch(&self, command: Command) {
        // A closed queue means the worker stopped (shutdown or a failed open); the view says so.
        let _ = self.inner.commands.send(command);
    }
    pub fn subscribe(&self) -> watch::Receiver<EditorView> {
        self.inner.view.clone()
    }
    pub fn view(&self) -> EditorView {
        self.inner.view.borrow().clone()
    }
}

type Sending = Pin<Box<dyn Future<Output = SendReceipt>>>;
struct Loop {
    handle: Handle,
    device: String,
    publish: watch::Sender<EditorView>,
    outcomes: u64,
    outcome: Option<Outcome>,
    sending: Option<Sending>,
}
async fn run(
    handle: Handle,
    device: String,
    mut queue: mpsc::UnboundedReceiver<Command>,
    publish: watch::Sender<EditorView>,
) {
    let mut session = match handle.open_editor(&device) {
        Ok(session) => session,
        Err(error) => {
            publish.send_modify(|view| view.open = OpenState::Failed(Failure::plain(error.into())));
            return;
        }
    };
    let mut state = Loop {
        handle: handle.clone(),
        device,
        publish,
        outcomes: 0,
        outcome: None,
        sending: None,
    };
    let stopping = handle.stopping();
    tokio::pin!(stopping);
    let mut listening = true;
    loop {
        state.show(&session);
        tokio::select! {
            biased;
            () = &mut stopping => break,
            command = queue.recv() => match command {
                Some(command) => state.apply(&mut session, command).await,
                None => break,
            },
            receipt = async { state.sending.as_mut().expect("guarded").await },
                if state.sending.is_some() =>
            {
                state.sending = None;
                // A failed transfer gets the Device-facing copy; refusals before it don't.
                let result = session.finish_send(receipt).map(|()| Done::Changed).map_err(|error| {
                    let notice = session.send_problem(&error);
                    Failure::new(error, notice)
                });
                state.record(CommandKind::Send, result);
            }
            change = session.changed(), if listening => {
                // The Companion is stopping; keep serving commands until the handles go.
                if let Err(SessionError::Send(_)) = change {
                    listening = false;
                }
            }
        }
    }
    // The Session's `Drop` cancels GIF work. Drop it before the view's sender, so a closed
    // view means that cleanup already happened.
    drop(session);
    drop(state);
}
impl Loop {
    async fn apply(&mut self, session: &mut Session, command: Command) {
        let kind = command.kind();
        let result: Result<Done, SessionError> = match command {
            Command::Select { page, tile } => {
                session.select(&page, tile.as_deref());
                Ok(Done::Changed)
            }
            Command::Orientation(orientation) => {
                session.select_orientation(orientation);
                match session.load().await {
                    // An orientation the Device has no Layout for keeps its draft, if any.
                    Ok(()) | Err(SessionError::Layout(LayoutError::NotFound)) => Ok(Done::Changed),
                    Err(error) => Err(error),
                }
            }
            Command::Edit(edit) => session
                .edit(edit)
                .map(|tile| tile.map_or(Done::Changed, Done::Tile))
                .map_err(Into::into),
            Command::DiscardRejected => session
                .discard_rejected()
                .map(|()| Done::Changed)
                .map_err(Into::into),
            Command::Add { kind, target } => session
                .add(kind, target)
                .map(Done::Tile)
                .map_err(Into::into),
            Command::AddGif {
                file,
                bytes,
                target,
            } => session
                .add_gif(file, bytes, target)
                .map(Done::Tile)
                .map_err(Into::into),
            Command::Gif { tile, edit } => session
                .gif(&tile, edit)
                .map(|()| Done::Changed)
                .map_err(Into::into),
            Command::Resolve(decision) => session
                .resolve(decision)
                .map(|()| Done::Changed)
                .map_err(Into::into),
            Command::Send => match self.begin_send(session) {
                // The outcome lands with the receipt; edits stay live meanwhile.
                Ok(pending) => {
                    self.sending = Some(Box::pin(pending.wait()));
                    return;
                }
                Err(error) => Err(error.into()),
            },
            Command::NewLayout(kind) => session
                .new_layout(kind)
                .map(|()| Done::Changed)
                .map_err(Into::into),
            Command::Import { file, bytes } => {
                session.import(&file, &bytes).map(|()| Done::Changed)
            }
            Command::Export => session.export().map(Done::Export).map_err(Into::into),
            Command::Load => session.load().await.map(|()| Done::Changed),
        };
        self.record(kind, result.map_err(Failure::plain));
    }
    /// The worker, not the screen, reads what the Device supports.
    fn begin_send(&self, session: &mut Session) -> Result<PendingSend, EditorError> {
        let sinks = DeviceId::parse(&self.device)
            .and_then(|id| self.handle.devices().get(&id))
            .and_then(|device| device.diagnostics)
            .map(|diagnostics| diagnostics.sinks);
        session.begin_send(Capabilities {
            sinks: sinks.as_deref(),
        })
    }
    fn record(&mut self, command: CommandKind, result: Result<Done, Failure>) {
        self.outcomes += 1;
        self.outcome = Some(Outcome {
            number: self.outcomes,
            command,
            result,
        });
    }
    fn show(&self, session: &Session) {
        let next = self.view(session);
        self.publish.send_if_modified(|view| {
            let changed = *view != next;
            if changed {
                *view = next;
            }
            changed
        });
    }
    fn view(&self, session: &Session) -> EditorView {
        let device = DeviceId::parse(&self.device).and_then(|id| self.handle.devices().get(&id));
        let draft = session.draft();
        let layout = draft.map(|draft| LayoutView {
            name: draft.layout().name.clone(),
            full_screen: draft.layout().full_screen,
            pages: draft
                .layout()
                .pages
                .iter()
                .map(|page| match page {
                    Page::Grid {
                        id,
                        title,
                        rows,
                        cols,
                        tiles,
                    } => PageView {
                        id: id.clone(),
                        title: title.clone(),
                        usage: false,
                        rows: *rows,
                        cols: *cols,
                        tiles: tiles
                            .iter()
                            .filter_map(|tile| {
                                Some(TileView {
                                    tile: draft.effective_tile(&tile.id)?,
                                    problems: draft
                                        .rejected()
                                        .iter()
                                        .filter(|r| refused_tile(r) == Some(tile.id.as_str()))
                                        .cloned()
                                        .collect(),
                                    gif: session.gif_work(&tile.id).cloned(),
                                    gif_fit: session.gif_fit(&tile.id),
                                })
                            })
                            .collect(),
                    },
                    Page::Dashboard { id, title, .. } => PageView {
                        id: id.clone(),
                        title: title.clone(),
                        usage: true,
                        rows: 0,
                        cols: 0,
                        tiles: Vec::new(),
                    },
                })
                .collect(),
        });
        let (page, tile) = session.selection();
        EditorView {
            open: OpenState::Ready,
            device: self.device.clone(),
            device_name: session.device_name(),
            orientation: session.orientation(),
            save_state: session.save_state(),
            layout,
            page: page.map(str::to_owned),
            tile: tile.map(str::to_owned),
            conflict: draft.and_then(|d| d.conflict()).map(|copy| ConflictView {
                name: copy.layout().name.clone(),
            }),
            add_target: session.add_target().ok(),
            push_progress: session.push_progress(),
            offers_gif: session.offers_gif(),
            control_layout: device.map(|d| d.control_layout).unwrap_or_default(),
            outcome: self.outcome.clone(),
        }
    }
}
fn refused_tile(rejected: &Rejected) -> Option<&str> {
    match &rejected.edit {
        Edit::Label { tile, .. } | Edit::Action { tile, .. } => Some(tile),
        _ => None,
    }
}
