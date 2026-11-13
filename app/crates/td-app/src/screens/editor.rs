//! The Layout editor, fed from one Device's editor `Worker` (td-companion's `editor`). The
//! Worker owns the Session on its own thread and publishes an `EditorView`; this module maps
//! that view to the window's properties and sends each callback back as one `Command`. Nothing
//! here waits for the Session, the disk or the Device: `dispatch` only queues.
use super::{Screens, opening, sync};
use crate::{
    AppWindow, CanvasCell, CanvasTile, DropTarget, EditorDevice, EditorState, FilmPage, Page,
    ThumbTile,
};
use slint::{ModelRc, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
use td_companion::{
    devices::DeviceSummary,
    editor::{
        AddKind, AddTarget, Command, CommandKind, Decision, EditorError, EditorView, Failure,
        LayoutView, NewLayout, OpenState, Outcome, PageView, Rejected, SaveState, SessionError,
        Worker,
    },
    layouts::LayoutError,
};
use td_core::{
    editing::{Edit, EditError},
    layout::Page as LayoutPage,
    live::{ScreenSize, cell_at, cell_rect},
};

/// A Layout holds up to this many Pages (`layout.page-count`).
pub(crate) const MAX_PAGES: usize = 8;
/// The Waveshare 5B panel, for a Device that hasn't reported its screen yet.
const DEFAULT_SCREEN: ScreenSize = ScreenSize { w: 1024, h: 600 };
/// How long quitting waits for each editor to save its queued edits.
pub(crate) const CLOSE_LIMIT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Models {
    pages: Rc<VecModel<FilmPage>>,
    thumbs: Rc<VecModel<ThumbTile>>,
    tiles: Rc<VecModel<CanvasTile>>,
    cells: Rc<VecModel<CanvasCell>>,
    devices: Rc<VecModel<EditorDevice>>,
}

/// The window's side of the editor: which Worker it shows and what it last showed.
#[derive(Default)]
pub(super) struct Editor {
    worker: RefCell<Option<Worker>>,
    /// Every Worker this window opened, so quitting waits for each to save.
    opened: RefCell<Vec<Worker>>,
    opens: Cell<u64>,
    /// Opening failed before the Worker existed (no runtime, no thread).
    open_error: RefCell<Option<String>>,
    /// The view and screen the properties were last built from, for the pure callbacks.
    shown: RefCell<Option<(EditorView, ScreenSize)>>,
    /// The first Ready view was checked for a draft.
    checked: Cell<bool>,
    /// A Load is out: the outcome number it has to pass.
    reading: Cell<Option<u64>>,
    /// The last Send was refused because the Device's Layout changed, so Keep mine sends again.
    resend: Cell<bool>,
    seen: Cell<u64>,
    models: Models,
}

impl Screens {
    pub(super) fn wire_editor(self: &Rc<Self>, ui: &AppWindow) {
        let models = &self.editor.models;
        ui.set_editor_pages(ModelRc::from(models.pages.clone()));
        ui.set_editor_thumbs(ModelRc::from(models.thumbs.clone()));
        ui.set_editor_tiles(ModelRc::from(models.tiles.clone()));
        ui.set_editor_cells(ModelRc::from(models.cells.clone()));
        ui.set_editor_devices(ModelRc::from(models.devices.clone()));
        ui.set_filmstrip_folded(self.window.filmstrip_folded());

        on!(self, ui, on_editor_shown, |s| s.refresh());
        on!(self, ui, on_editor_set_folded, |s, folded: bool| {
            s.window.set_filmstrip_folded(folded, &s.runtime);
            if let Some(ui) = s.ui.upgrade() {
                ui.set_filmstrip_folded(folded);
            }
        });
        on!(self, ui, on_editor_choose_device, |s, id: SharedString| s
            .choose_device(&id));
        on!(self, ui, on_editor_read, |s| s.read_layout());
        on!(self, ui, on_editor_start, |s, starter: bool| s.dispatch(
            Command::NewLayout(if starter {
                NewLayout::Starter
            } else {
                NewLayout::Blank
            })
        ));
        on!(self, ui, on_editor_select_page, |s, id: SharedString| s
            .dispatch(Command::Select {
                page: id.into(),
                tile: None,
            }));
        on!(self, ui, on_editor_add_page, |s| {
            if let Some(edit) = s.with_shown(|view, _| add_page(view)).flatten() {
                s.dispatch(Command::Edit(edit));
            }
        });
        on!(self, ui, on_editor_remove_page, |s, id: SharedString| s
            .dispatch(Command::Edit(Edit::RemovePage {
                page: id.into()
            })));
        on!(self, ui, on_editor_select_tile, |s, id: SharedString| {
            if let Some(page) = s.with_shown(|view, _| view.page.clone()).flatten() {
                s.dispatch(Command::Select {
                    page,
                    tile: Some(id.into()),
                });
            }
        });
        on!(self, ui, on_editor_clear_selection, |s| {
            if let Some(page) = s.with_shown(|view, _| view.page.clone()).flatten() {
                s.dispatch(Command::Select { page, tile: None });
            }
        });
        on!(
            self,
            ui,
            on_editor_activate_cell,
            |s, row: i32, col: i32| {
                if let Some(command) = s.with_shown(|view, _| activate(view, row, col)).flatten() {
                    s.dispatch(command);
                }
            }
        );
        on!(self, ui, on_editor_add_tile, |s, row: i32, col: i32| {
            if let (Ok(row), Ok(col)) = (u8::try_from(row), u8::try_from(col)) {
                s.dispatch(Command::Add {
                    kind: AddKind::Key,
                    target: AddTarget::Cell { row, col },
                });
            }
        });
        on!(
            self,
            ui,
            on_editor_move_tile,
            |s, id: SharedString, row: i32, col: i32| {
                if let Some(edit) = s.with_shown(|view, _| place(view, &id, row, col)).flatten() {
                    s.dispatch(Command::Edit(edit));
                }
            }
        );
        on!(
            self,
            ui,
            on_editor_resize_tile,
            |s, id: SharedString, rows: i32, cols: i32| {
                if let Some(edit) = s
                    .with_shown(|view, _| span(view, &id, rows, cols))
                    .flatten()
                {
                    s.dispatch(Command::Edit(edit));
                }
            }
        );
        on!(self, ui, on_editor_nudge, |s, dx: i32, dy: i32| {
            if let Some(edit) = s.with_shown(|view, _| nudge(view, dx, dy)).flatten() {
                s.dispatch(Command::Edit(edit));
            }
        });
        on!(self, ui, on_editor_grow, |s, dx: i32, dy: i32| {
            if let Some(edit) = s.with_shown(|view, _| grow(view, dx, dy)).flatten() {
                s.dispatch(Command::Edit(edit));
            }
        });
        on!(self, ui, on_editor_remove_tile, |s| {
            if let Some(tile) = s.with_shown(|view, _| view.tile.clone()).flatten() {
                s.dispatch(Command::Edit(Edit::RemoveTile { tile }));
            }
        });
        let weak = Rc::downgrade(self);
        ui.on_editor_drop_target(move |id, press_x, press_y, x, y, resize| {
            weak.upgrade()
                .and_then(|s| {
                    s.with_shown(|view, screen| {
                        drop_target(view, screen, &id, (press_x, press_y), (x, y), resize)
                    })
                })
                .unwrap_or_default()
        });
        on!(self, ui, on_editor_send, |s| s.dispatch(Command::Send));
        on!(self, ui, on_editor_keep_mine, |s| {
            s.dispatch(Command::Resolve(Decision::KeepMine));
            // Keep mine after a refused send sends again.
            if s.editor.resend.replace(false) {
                s.dispatch(Command::Send);
            }
        });
        on!(self, ui, on_editor_replace, |s| {
            s.editor.resend.set(false);
            s.dispatch(Command::Resolve(Decision::UseDevice));
        });
    }

    fn dispatch(&self, command: Command) {
        if let Some(worker) = self.editor.worker.borrow().as_ref() {
            worker.dispatch(command);
        }
    }

    fn with_shown<T>(&self, read: impl FnOnce(&EditorView, ScreenSize) -> T) -> Option<T> {
        let shown = self.editor.shown.borrow();
        shown.as_ref().map(|(view, screen)| read(view, *screen))
    }

    /// Reads the Device's Layout into the editor.
    fn read_layout(&self) {
        let Some(worker) = self.editor.worker.borrow().clone() else {
            return;
        };
        let at = worker.view().outcome.map_or(0, |o| o.number);
        self.editor.reading.set(Some(at));
        worker.dispatch(Command::Load);
        self.refresh();
    }

    /// The Device the editor opens on: the last one edited if it's still known, else the
    /// first connected one, else the first.
    fn editor_choice(&self, devices: &[DeviceSummary]) -> Option<String> {
        let last = self.window.editor_device();
        devices
            .iter()
            .find(|d| last.as_deref() == Some(d.id.as_str()))
            .or_else(|| devices.iter().find(|d| super::connected(d)))
            .or_else(|| devices.first())
            .map(|d| d.id.as_str().to_owned())
    }

    fn choose_device(&self, id: &str) {
        let current = self
            .editor
            .worker
            .borrow()
            .as_ref()
            .map(|w| w.view().device);
        if current.as_deref() == Some(id) {
            return;
        }
        if let Some(old) = self.editor.worker.borrow_mut().take() {
            // It saves its queued edits on its own thread; quitting still waits for it.
            self.runtime.spawn(async move { old.close().await });
        }
        self.open_editor(id);
        self.refresh();
    }

    fn open_editor(&self, id: &str) {
        let opened = {
            // The Worker takes the runtime from the caller's context.
            let _runtime = self.runtime.enter();
            self.handle.open_editor_worker(id)
        };
        let editor = &self.editor;
        editor.checked.set(false);
        editor.reading.set(None);
        editor.resend.set(false);
        editor.seen.set(0);
        match opened {
            Ok(worker) => {
                editor.opens.set(editor.opens.get() + 1);
                self.watch(format!("editor:{}", editor.opens.get()), worker.subscribe());
                editor.opened.borrow_mut().push(worker.clone());
                *editor.worker.borrow_mut() = Some(worker);
                *editor.open_error.borrow_mut() = None;
                self.window.set_editor_device(id, &self.runtime);
            }
            Err(error) => *editor.open_error.borrow_mut() = Some(error.to_string()),
        }
    }

    /// Every editor Worker this window opened, to close before the Companion stops.
    pub(crate) fn editor_workers(&self) -> Vec<Worker> {
        self.editor.opened.borrow().clone()
    }

    pub(super) fn refresh_editor(&self, ui: &AppWindow, devices: &[DeviceSummary]) {
        let editor = &self.editor;
        sync(
            &editor.models.devices,
            devices
                .iter()
                .map(|d| EditorDevice {
                    id: d.id.as_str().into(),
                    name: super::shown_name(d).into(),
                })
                .collect(),
        );
        if ui.get_page() == Page::Layout
            && editor.worker.borrow().is_none()
            && let Some(id) = self.editor_choice(devices)
        {
            self.open_editor(&id);
        }
        let Some(worker) = editor.worker.borrow().clone() else {
            let (message, detail) = match editor.open_error.borrow().as_ref() {
                Some(error) => (
                    "Couldn't open the editor.".to_owned(),
                    format!("Restart Open TouchDeck, then retry. ({error})"),
                ),
                None => (
                    "Plug in your Device to edit its Layout.".to_owned(),
                    String::new(),
                ),
            };
            self.show_editor(
                ui,
                Mapped::empty(EditorState {
                    state: if editor.open_error.borrow().is_some() {
                        "failed"
                    } else {
                        "no-device"
                    }
                    .into(),
                    message: message.into(),
                    detail: detail.into(),
                    aspect: aspect(DEFAULT_SCREEN),
                    ..Default::default()
                }),
            );
            return;
        };
        let view = worker.view();
        self.follow_outcome(&view);
        if view.open == OpenState::Ready && !editor.checked.replace(true) && view.layout.is_none() {
            // No draft yet: start from what the Device shows.
            editor
                .reading
                .set(Some(view.outcome.as_ref().map_or(0, |o| o.number)));
            worker.dispatch(Command::Load);
        }
        let device = devices.iter().find(|d| d.id.as_str() == view.device);
        let screen = screen_of(device, &view);
        let name = device.map_or_else(|| view.device_name.clone(), super::shown_name);
        let reading = editor.reading.get().is_some();
        self.show_editor(ui, map(&view, screen, &name, reading));
        *editor.shown.borrow_mut() = Some((view, screen));
    }

    /// Acts once on each new outcome.
    fn follow_outcome(&self, view: &EditorView) {
        let editor = &self.editor;
        let Some(outcome) = &view.outcome else {
            return;
        };
        if outcome.number <= editor.seen.replace(outcome.number) {
            return;
        }
        if outcome.command == CommandKind::Load
            && editor.reading.get().is_some_and(|at| outcome.number > at)
        {
            editor.reading.set(None);
        }
        if outcome.command == CommandKind::Send {
            editor.resend.set(outcome.result.as_ref().is_err_and(|f| {
                f.refusal()
                    .is_some_and(|refusal| refusal.code == "layout.changed")
            }));
        }
    }

    fn show_editor(&self, ui: &AppWindow, mapped: Mapped) {
        let models = &self.editor.models;
        ui.set_editor(mapped.state);
        sync(&models.pages, mapped.pages);
        sync(&models.thumbs, mapped.thumbs);
        sync(&models.tiles, mapped.tiles);
        sync(&models.cells, mapped.cells);
    }
}

/// The Device's screen in the orientation being edited.
fn screen_of(device: Option<&DeviceSummary>, view: &EditorView) -> ScreenSize {
    device
        .and_then(|d| d.capabilities.as_ref())
        .and_then(|caps| {
            Some(ScreenSize {
                w: u16::try_from(caps.screen[0]).ok()?,
                h: u16::try_from(caps.screen[1]).ok()?,
            })
        })
        .filter(|s| s.w > 0 && s.h > 0)
        .unwrap_or(DEFAULT_SCREEN)
        .oriented(view.orientation)
}

fn aspect(screen: ScreenSize) -> f32 {
    f32::from(screen.w) / f32::from(screen.h.max(1))
}

/// What the window shows for one view.
#[derive(Debug, Default)]
pub(crate) struct Mapped {
    pub state: EditorState,
    pub pages: Vec<FilmPage>,
    pub thumbs: Vec<ThumbTile>,
    pub tiles: Vec<CanvasTile>,
    pub cells: Vec<CanvasCell>,
}
impl Mapped {
    fn empty(state: EditorState) -> Self {
        Self {
            state,
            ..Default::default()
        }
    }
}

/// The current Page and its index, falling back to the first.
fn current(view: &EditorView) -> Option<(&LayoutView, usize, &PageView)> {
    let layout = view.layout.as_ref()?;
    let index = layout
        .pages
        .iter()
        .position(|p| Some(&p.id) == view.page.as_ref())
        .unwrap_or(0);
    Some((layout, index, layout.pages.get(index)?))
}

/// A span's rect as fractions of the screen.
fn fraction(screen: ScreenSize, full: bool, grid: [u8; 2], cell: [u8; 4]) -> Option<[f32; 4]> {
    let rect = cell_rect(screen, full, grid, cell)?;
    let (w, h) = (f32::from(screen.w), f32::from(screen.h));
    Some([
        rect.x as f32 / w,
        rect.y as f32 / h,
        f32::from(rect.w) / w,
        f32::from(rect.h) / h,
    ])
}

/// Maps a view to the window's properties. `name` is the Device's name as the window shows it;
/// `reading` says a Load the window asked for is still out.
pub(crate) fn map(view: &EditorView, screen: ScreenSize, name: &str, reading: bool) -> Mapped {
    let base = EditorState {
        device_id: view.device.as_str().into(),
        device_name: name.into(),
        aspect: aspect(screen),
        ..Default::default()
    };
    let state = |state: &str, message: String, detail: String| EditorState {
        state: state.into(),
        message: message.into(),
        detail: detail.into(),
        ..base.clone()
    };
    let named = opening(name);
    match &view.open {
        OpenState::Opening => {
            return Mapped::empty(state(
                "opening",
                "Opening the editor…".into(),
                String::new(),
            ));
        }
        OpenState::Failed(failure) => {
            return Mapped::empty(state(
                "failed",
                "Couldn't open the Layout you were editing.".into(),
                format!(
                    "Open TouchDeck keeps your unsent changes on this PC and couldn't read them. Nothing was changed. ({})",
                    failure.error
                ),
            ));
        }
        OpenState::Ready => {}
    }
    let Some((layout, index, page)) = current(view) else {
        let load = view
            .outcome
            .as_ref()
            .filter(|o| !reading && o.command == CommandKind::Load);
        return Mapped::empty(match load.map(|o| &o.result) {
            Some(Err(failure))
                if matches!(*failure.error, SessionError::Layout(LayoutError::NotFound)) =>
            {
                state(
                    "empty",
                    format!("{named} has no Layout yet."),
                    format!("Start one here, then send it to {name}."),
                )
            }
            Some(Err(failure)) => state(
                "unreadable",
                format!("Couldn't read the Layout from {name}."),
                format!(
                    "Check that it's plugged in, then retry. You can also start a new Layout here. ({})",
                    failure.error
                ),
            ),
            _ => state(
                "reading",
                format!("Reading the Layout from {name}…"),
                String::new(),
            ),
        });
    };

    let full = layout.full_screen;
    let mut pages = Vec::new();
    let mut thumbs = Vec::new();
    for (n, p) in layout.pages.iter().enumerate() {
        let problem = p.tiles.iter().any(|t| !t.problems.is_empty());
        let mut label = format!("Page {} of {}, {}", n + 1, layout.pages.len(), p.title);
        if p.usage {
            label.push_str(", Usage");
        }
        if problem {
            label.push_str(", needs a fix");
        }
        if n == index {
            label.push_str(", current");
        }
        pages.push(FilmPage {
            id: p.id.as_str().into(),
            number: n as i32 + 1,
            title: p.title.as_str().into(),
            usage: p.usage,
            current: n == index,
            problem,
            label: label.into(),
        });
        for tile in &p.tiles {
            let t = &tile.tile;
            if let Some([x, y, w, h]) = fraction(
                screen,
                full,
                [p.rows, p.cols],
                [t.row, t.col, t.row_span, t.col_span],
            ) {
                thumbs.push(ThumbTile {
                    page: n as i32,
                    x,
                    y,
                    w,
                    h,
                    problem: !tile.problems.is_empty(),
                });
            }
        }
    }

    let mut tiles = Vec::new();
    for tile in &page.tiles {
        let t = &tile.tile;
        let Some([x, y, w, h]) = fraction(
            screen,
            full,
            [page.rows, page.cols],
            [t.row, t.col, t.row_span, t.col_span],
        ) else {
            continue;
        };
        let label = if t.label.is_empty() { "Tile" } else { &t.label };
        tiles.push(CanvasTile {
            id: t.id.as_str().into(),
            label: t.label.as_str().into(),
            x,
            y,
            w,
            h,
            row: t.row.into(),
            col: t.col.into(),
            selected: view.tile.as_deref() == Some(t.id.as_str()),
            problem: tile
                .problems
                .first()
                .map(problem_text)
                .unwrap_or_default()
                .into(),
            name: format!(
                "{label}, row {}, column {}, {} by {}",
                t.row + 1,
                t.col + 1,
                t.row_span,
                t.col_span
            )
            .into(),
        });
    }

    let mut cells = Vec::new();
    for row in 0..page.rows {
        for col in 0..page.cols {
            let Some([x, y, w, h]) =
                fraction(screen, full, [page.rows, page.cols], [row, col, 1, 1])
            else {
                continue;
            };
            let empty = !page.tiles.iter().any(|tile| {
                let t = &tile.tile;
                (t.row..t.row + t.row_span).contains(&row)
                    && (t.col..t.col + t.col_span).contains(&col)
            });
            cells.push(CanvasCell {
                row: row.into(),
                col: col.into(),
                x,
                y,
                w,
                h,
                empty,
                name: format!("Add Tile, row {}, column {}", row + 1, col + 1).into(),
            });
        }
    }

    let selected = view
        .tile
        .as_ref()
        .and_then(|id| page.tiles.iter().map(|t| &t.tile).find(|t| &t.id == id));
    let (save_state, caution) = match view.save_state {
        SaveState::None => ("", false),
        SaveState::Saved => ("Sent", false),
        SaveState::Unsent => ("Unsent changes", false),
        SaveState::Sending => ("Sending…", false),
        SaveState::Unconfirmed => ("Couldn't confirm", true),
    };
    let progress = view
        .push_progress
        .filter(|p| p.bytes_total > 0)
        .map(|p| (p.bytes_sent.min(p.bytes_total) * 100 / p.bytes_total) as i32);
    // The title band and pager, as the Firmware draws them (`td_core::live::cell_rect`).
    let side = u32::from(screen.w.min(screen.h));
    let height = f32::from(screen.h);
    let (title_h, pager_y) = if full {
        (0.0, 0.0)
    } else {
        (
            (56 * side / 600) as f32 / height,
            1.0 - (36 * side / 600) as f32 / height,
        )
    };
    let notice = view
        .outcome
        .as_ref()
        .and_then(|outcome| notice(outcome, view, name));
    let conflict = view.conflict.as_ref();
    Mapped {
        state: EditorState {
            state: "ready".into(),
            save_state: save_state.into(),
            save_caution: caution,
            sending: view.save_state == SaveState::Sending,
            progress: progress.unwrap_or(-1),
            progress_text: progress.map(|p| format!("{p}%")).unwrap_or_default().into(),
            page_id: page.id.as_str().into(),
            page_title: page.title.as_str().into(),
            page_usage: page.usage,
            page_index: index as i32,
            page_count: layout.pages.len() as i32,
            rows: page.rows.into(),
            cols: page.cols.into(),
            title_h,
            pager_y,
            tile_selected: selected.is_some(),
            selected_row: selected.map_or(0, |t| t.row.into()),
            selected_col: selected.map_or(0, |t| t.col.into()),
            notice: notice
                .as_ref()
                .map(|(text, _)| text.as_str())
                .unwrap_or_default()
                .into(),
            notice_detail: notice
                .as_ref()
                .map(|(_, detail)| detail.as_str())
                .unwrap_or_default()
                .into(),
            conflict: conflict.is_some(),
            conflict_title: conflict
                .map(|c| format!("{named} has \u{201c}{}\u{201d}", c.name))
                .unwrap_or_default()
                .into(),
            conflict_body: conflict
                .map(|c| {
                    format!(
                        "The Layout on {name} isn't the one you're editing. Keep mine keeps your changes. Replace drops them and edits \u{201c}{}\u{201d} instead.",
                        c.name
                    )
                })
                .unwrap_or_default()
                .into(),
            can_add_page: layout.pages.len() < MAX_PAGES,
            can_remove_page: layout.pages.len() > 1,
            canvas_description: if page.usage {
                format!("{}, Usage Page", page.title)
            } else {
                format!("{}, {} rows by {} columns", page.title, page.rows, page.cols)
            }
            .into(),
            ..base
        },
        pages,
        thumbs,
        tiles,
        cells,
    }
}

/// The line shown above the Device for the last command, if it failed. A refused send that
/// opened the conflict dialog needs no second message.
fn notice(outcome: &Outcome, view: &EditorView, name: &str) -> Option<(String, String)> {
    let failure = outcome.result.as_ref().err()?;
    let refusal = failure.refusal();
    let code = refusal.as_ref().map(|r| r.code.as_str());
    if view.conflict.is_some() && code == Some("layout.changed") {
        return None;
    }
    if outcome.command == CommandKind::Load
        && matches!(*failure.error, SessionError::Layout(LayoutError::NotFound))
    {
        return None;
    }
    let detail = if failure.notice.detail.is_empty() {
        failure.error.to_string()
    } else {
        failure.notice.detail.clone()
    };
    Some((failure_text(failure, outcome.command, name), detail))
}

/// Words for a refused command. The Session's own copy is used where it was written for
/// people: a send that failed on its way to the Device, and an Import.
pub(crate) fn failure_text(failure: &Failure, command: CommandKind, name: &str) -> String {
    let named = opening(name);
    match &*failure.error {
        SessionError::Editor(error) => editor_text(error, name),
        SessionError::Import { .. } => failure.error.to_string(),
        SessionError::Layout(_)
            if matches!(command, CommandKind::Load | CommandKind::Orientation) =>
        {
            format!("Couldn't read the Layout from {name}. Check that it's plugged in, then retry.")
        }
        _ if command == CommandKind::Send && !failure.notice.text.is_empty() => {
            failure.notice.text.clone()
        }
        SessionError::Layout(_) | SessionError::Job(_) | SessionError::Send(_) => {
            format!("{named} didn't take that change. Check that it's plugged in, then retry.")
        }
        SessionError::Asset(_) => "Couldn't prepare that image. Nothing changed.".into(),
    }
}

fn editor_text(error: &EditorError, name: &str) -> String {
    let named = opening(name);
    match error {
        EditorError::Edit(error) => edit_text(error),
        EditorError::Rejected => {
            "Fix the Tiles marked with a caution sign, then send again.".into()
        }
        EditorError::Conflict => {
            format!("Choose which Layout to keep, then send to {name} again.")
        }
        EditorError::Sending => format!("Wait until sending to {name} has finished."),
        EditorError::NoDraft => "Start a Layout first.".into(),
        EditorError::Io(_) => {
            "Couldn't save your changes on this PC. Check that the disk has space, then retry."
                .into()
        }
        EditorError::UnknownCapabilities => {
            format!("Connect {name}, then send again.")
        }
        EditorError::Incompatible(_) => format!(
            "{named}'s Firmware can't show this Layout. Update its Firmware, then send again."
        ),
        EditorError::Stopped => "Open TouchDeck closed before this change ran.".into(),
        EditorError::PushMismatch => {
            format!("{named} didn't show the Layout that was sent. Send it again.")
        }
        EditorError::StalePush => "The Layout changed while it was sending. Send it again.".into(),
        EditorError::InvalidDevice | EditorError::RuntimeUnavailable => {
            "The editor can't work right now. Restart Open TouchDeck, then retry.".into()
        }
    }
}

/// A structural refusal changes nothing, so it says so.
fn edit_text(error: &EditError) -> String {
    let rule = match error {
        EditError::NoSpace => {
            return "There's no room for that on this Page. Nothing changed.".into();
        }
        EditError::MissingPage(_) | EditError::MissingTile(_) | EditError::NotGrid(_) => {
            return "That Page or Tile is gone. Nothing changed.".into();
        }
        EditError::SinkNotApplicable => {
            return "Only keys and media keys can go to a computer or phone.".into();
        }
        EditError::LiveActionRequired | EditError::BarRequired => {
            return "That look needs a different kind of Tile. Nothing changed.".into();
        }
        EditError::Invalid(problems) => problems.first().map(|p| p.rule.as_str()),
    };
    match rule_text(rule.unwrap_or_default()) {
        Some(text) => format!("{text} Nothing changed."),
        None => "That change breaks a Layout rule. Nothing changed.".into(),
    }
}

/// The message under a refused Label or Action, which stays in its field.
pub(crate) fn problem_text(rejected: &Rejected) -> String {
    let rule = match &rejected.reason {
        EditError::Invalid(problems) => problems.first().map(|p| p.rule.as_str()),
        _ => None,
    };
    rule.and_then(rule_text)
        .map(str::to_owned)
        .unwrap_or_else(|| "This value breaks a Layout rule.".into())
}

fn rule_text(rule: &str) -> Option<&'static str> {
    Some(match rule {
        "label.too-long" => "Tile labels can be up to 16 characters.",
        "label.blank" => "Give the Tile a name.",
        "title.blank" => "Give the Page a name.",
        "title.too-long" => "That Page name is too long.",
        "name.blank" => "Give the Layout a name.",
        "name.too-long" => "That Layout name is too long.",
        "layout.page-count" => "A Layout holds up to 8 Pages.",
        "page.tile-count" => "This Page can't hold more Tiles.",
        "tile.bounds" | "tile.position" | "tile.span" => "That doesn't fit inside the grid.",
        "tile.overlap" => "Another Tile is in the way.",
        "grid.size" => "A grid has 1 to 8 rows and 1 to 8 columns.",
        "action.page-missing" => "That Tile opens a Page that isn't in this Layout.",
        "text.empty" => "Type the text this Tile sends.",
        "key.usage-range" | "consumer.usage-range" | "modifier.unknown" => {
            "Choose a key from the list."
        }
        _ => return None,
    })
}

/// A new Page after the last, with the current Page's grid.
fn add_page(view: &EditorView) -> Option<Edit> {
    let (layout, _, page) = current(view)?;
    if layout.pages.len() >= MAX_PAGES {
        return None;
    }
    let n = (layout.pages.len() + 1..)
        .find(|n| !layout.pages.iter().any(|p| p.id == format!("page{n}")))?;
    let (rows, cols) = if page.usage {
        (3, 4)
    } else {
        (page.rows, page.cols)
    };
    Some(Edit::AddPage {
        page: LayoutPage::Grid {
            id: format!("page{n}"),
            title: format!("Page {n}"),
            rows,
            cols,
            tiles: Vec::new(),
        },
    })
}

/// Enter on a cell: select its Tile, or add one there.
fn activate(view: &EditorView, row: i32, col: i32) -> Option<Command> {
    let (_, _, page) = current(view)?;
    let (row, col) = (u8::try_from(row).ok()?, u8::try_from(col).ok()?);
    let found = page.tiles.iter().map(|t| &t.tile).find(|t| {
        (t.row..t.row + t.row_span).contains(&row) && (t.col..t.col + t.col_span).contains(&col)
    });
    Some(match found {
        Some(tile) => Command::Select {
            page: page.id.clone(),
            tile: Some(tile.id.clone()),
        },
        None => Command::Add {
            kind: AddKind::Key,
            target: AddTarget::Cell { row, col },
        },
    })
}

fn tile<'a>(view: &'a EditorView, id: &str) -> Option<(&'a PageView, &'a td_core::layout::Tile)> {
    let (_, _, page) = current(view)?;
    let tile = page.tiles.iter().map(|t| &t.tile).find(|t| t.id == id)?;
    Some((page, tile))
}

/// A Move to `row`, `col`, unless the Tile is already there.
fn place(view: &EditorView, id: &str, row: i32, col: i32) -> Option<Edit> {
    let (_, tile) = tile(view, id)?;
    let (row, col) = (u8::try_from(row).ok()?, u8::try_from(col).ok()?);
    ((row, col) != (tile.row, tile.col)).then(|| Edit::Move {
        tile: id.into(),
        row,
        col,
    })
}

fn span(view: &EditorView, id: &str, rows: i32, cols: i32) -> Option<Edit> {
    let (_, tile) = tile(view, id)?;
    let (rows, cols) = (u8::try_from(rows).ok()?, u8::try_from(cols).ok()?);
    ((rows, cols) != (tile.row_span, tile.col_span) && rows > 0 && cols > 0).then(|| Edit::Resize {
        tile: id.into(),
        row_span: rows,
        col_span: cols,
    })
}

/// An arrow key moves the selected Tile one cell; at the grid's edge it stays.
fn nudge(view: &EditorView, dx: i32, dy: i32) -> Option<Edit> {
    let (page, tile) = tile(view, view.tile.as_deref()?)?;
    let row = i32::from(tile.row) + dy;
    let col = i32::from(tile.col) + dx;
    let fits = row >= 0
        && col >= 0
        && row + i32::from(tile.row_span) <= i32::from(page.rows)
        && col + i32::from(tile.col_span) <= i32::from(page.cols);
    if !fits {
        return None;
    }
    place(view, &tile.id, row, col)
}

/// Shift+arrow grows or shrinks the selected Tile from its bottom-right corner.
fn grow(view: &EditorView, dx: i32, dy: i32) -> Option<Edit> {
    let (page, tile) = tile(view, view.tile.as_deref()?)?;
    let rows = i32::from(tile.row_span) + dy;
    let cols = i32::from(tile.col_span) + dx;
    let fits = rows >= 1
        && cols >= 1
        && i32::from(tile.row) + rows <= i32::from(page.rows)
        && i32::from(tile.col) + cols <= i32::from(page.cols);
    if !fits {
        return None;
    }
    span(view, &tile.id, rows, cols)
}

/// Where a drag would put the Tile: the pointer's cell under the cell it was grabbed by, or,
/// for the corner handle, the span reaching the pointer's cell. Points are fractions of the
/// screen. Not `ok` outside the grid or over another Tile.
pub(crate) fn drop_target(
    view: &EditorView,
    screen: ScreenSize,
    id: &str,
    press: (f32, f32),
    at: (f32, f32),
    resize: bool,
) -> DropTarget {
    let Some((layout, _, page)) = current(view) else {
        return DropTarget::default();
    };
    let Some((_, tile)) = tile(view, id) else {
        return DropTarget::default();
    };
    let grid = [page.rows, page.cols];
    let cell = |(x, y): (f32, f32)| {
        cell_at(
            screen,
            layout.full_screen,
            grid,
            f64::from(x) * f64::from(screen.w),
            f64::from(y) * f64::from(screen.h),
        )
    };
    let Some([at_row, at_col]) = cell(at) else {
        return DropTarget::default();
    };
    let (top, left) = (i32::from(tile.row), i32::from(tile.col));
    let (row, col, rows, cols) = if resize {
        (top, left, at_row - top + 1, at_col - left + 1)
    } else {
        let Some([grab_row, grab_col]) = cell(press) else {
            return DropTarget::default();
        };
        let within =
            |n: i32, start: i32, len: u8| n.clamp(start, start + i32::from(len) - 1) - start;
        (
            at_row - within(grab_row, top, tile.row_span),
            at_col - within(grab_col, left, tile.col_span),
            i32::from(tile.row_span),
            i32::from(tile.col_span),
        )
    };
    let inside = row >= 0
        && col >= 0
        && rows >= 1
        && cols >= 1
        && row + rows <= i32::from(page.rows)
        && col + cols <= i32::from(page.cols);
    let clear = !page.tiles.iter().map(|t| &t.tile).any(|other| {
        other.id != tile.id
            && row < i32::from(other.row + other.row_span)
            && i32::from(other.row) < row + rows
            && col < i32::from(other.col + other.col_span)
            && i32::from(other.col) < col + cols
    });
    let rect = (inside && clear)
        .then(|| {
            fraction(
                screen,
                layout.full_screen,
                grid,
                [row as u8, col as u8, rows as u8, cols as u8],
            )
        })
        .flatten();
    match rect {
        Some([x, y, w, h]) => DropTarget {
            ok: true,
            row,
            col,
            row_span: rows,
            col_span: cols,
            x,
            y,
            w,
            h,
        },
        None => DropTarget::default(),
    }
}
