use super::*;
use crate::screens::{MAX_PAGES, drop_target, failure_text, map, problem_text};
use std::sync::Arc;
use td_companion::{
    checkpoints::Checkpoint,
    editor::{
        Command, CommandKind, ConflictView, EditorError, EditorView, Failure, LayoutView, Notice,
        OpenState, Outcome, PageView, Rejected, SaveState, SessionError, TileView,
    },
    layouts::PushProgress,
};
use td_core::{
    Orientation,
    editing::{Edit, EditError, Problem},
    layout::{Action, Tile},
    live::{ScreenSize, cell_rect},
};

const SCREEN: ScreenSize = ScreenSize { w: 1024, h: 600 };

fn tile(id: &str, label: &str, row: u8, col: u8, row_span: u8, col_span: u8) -> TileView {
    TileView {
        tile: Tile {
            id: id.into(),
            label: label.into(),
            row,
            col,
            row_span,
            col_span,
            icon: None,
            asset: None,
            render: None,
            streams: None,
            input: false,
            action: Action::None,
        },
        problems: Vec::new(),
        gif: None,
        gif_fit: Default::default(),
    }
}

fn grid(id: &str, title: &str, tiles: Vec<TileView>) -> PageView {
    PageView {
        id: id.into(),
        title: title.into(),
        usage: false,
        rows: 3,
        cols: 4,
        tiles,
    }
}

fn view(pages: Vec<PageView>) -> EditorView {
    EditorView {
        open: OpenState::Ready,
        device: "mock0001".into(),
        device_name: "Desk".into(),
        orientation: Orientation::Landscape,
        save_state: SaveState::Unsent,
        page: pages.first().map(|p| p.id.clone()),
        layout: Some(LayoutView {
            name: "Mine".into(),
            full_screen: false,
            pages,
        }),
        tile: None,
        conflict: None,
        add_target: None,
        push_progress: None,
        offers_gif: false,
        control_layout: Default::default(),
        outcome: None,
    }
}

fn failed(command: CommandKind, error: SessionError, copy: &str) -> Outcome {
    Outcome {
        number: 1,
        command,
        result: Err(Failure {
            notice: Notice {
                text: copy.into(),
                detail: String::new(),
            },
            error: Arc::new(error),
        }),
    }
}

fn too_long(tile: &str) -> Rejected {
    Rejected {
        edit: Edit::Label {
            tile: tile.into(),
            label: "Seventeen letters".into(),
        },
        reason: EditError::Invalid(vec![Problem {
            rule: "label.too-long".into(),
            path: "/pages/0/tiles/0/label".into(),
            message: "label is longer than 16 characters".into(),
        }]),
    }
}

#[test]
fn a_view_maps_to_the_filmstrip_canvas_and_header() {
    let mut spotify = tile("spotify", "Spotify", 0, 1, 1, 2);
    spotify.problems.push(too_long("spotify"));
    let mut usage = grid("usage", "Usage", Vec::new());
    usage.usage = true;
    let mut shown = view(vec![
        grid("main", "Main", vec![tile("mute", "Mute", 0, 0, 1, 1)]),
        grid("media", "Media", vec![spotify]),
        usage,
    ]);
    shown.page = Some("media".into());
    shown.tile = Some("spotify".into());
    shown.save_state = SaveState::Sending;
    shown.push_progress = Some(PushProgress {
        assets_sent: 1,
        assets_total: 2,
        bytes_sent: 250,
        bytes_total: 1000,
    });

    let mapped = map(&shown, SCREEN, "Desk", false);
    let state = &mapped.state;
    assert_eq!(state.state, "ready");
    assert_eq!(state.device_name, "Desk");
    assert!((state.aspect - 1024.0 / 600.0).abs() < 1e-6);
    assert_eq!(state.save_state, "Sending…");
    assert!(state.sending);
    assert_eq!((state.progress, state.progress_text.as_str()), (25, "25%"));
    assert_eq!((state.page_index, state.page_count), (1, 3));
    assert_eq!(state.page_title, "Media");
    assert!(state.tile_selected);
    assert_eq!((state.selected_row, state.selected_col), (0, 1));
    assert_eq!(state.canvas_description, "Media, 3 rows by 4 columns");
    assert!(state.can_add_page && state.can_remove_page);
    assert_eq!(state.notice, "");
    // The title band and pager, as the Firmware draws them.
    assert!((state.title_h - 56.0 / 600.0).abs() < 1e-6);
    assert!((state.pager_y - (1.0 - 36.0 / 600.0)).abs() < 1e-6);

    let labels: Vec<_> = mapped.pages.iter().map(|p| p.label.to_string()).collect();
    assert_eq!(
        labels,
        [
            "Page 1 of 3, Main",
            "Page 2 of 3, Media, needs a fix, current",
            "Page 3 of 3, Usage, Usage",
        ]
    );
    assert!(mapped.pages[2].usage);
    assert!(mapped.pages[1].problem && mapped.pages[1].current);

    // Thumbnails draw every Page's Tiles; the canvas only the current Page's.
    assert_eq!(mapped.thumbs.len(), 2);
    assert_eq!(mapped.thumbs[1].page, 1);
    assert!(mapped.thumbs[1].problem);
    assert_eq!(mapped.tiles.len(), 1);
    let canvas = &mapped.tiles[0];
    assert_eq!(canvas.name, "Spotify, row 1, column 2, 1 by 2");
    assert_eq!(canvas.problem, "Tile labels can be up to 16 characters.");
    assert!(canvas.selected);
    let rect = cell_rect(SCREEN, false, [3, 4], [0, 1, 1, 2]).unwrap();
    assert!((canvas.x - rect.x as f32 / 1024.0).abs() < 1e-6);
    assert!((canvas.w - f32::from(rect.w) / 1024.0).abs() < 1e-6);
    assert!((canvas.y - rect.y as f32 / 600.0).abs() < 1e-6);

    // Twelve cells; the two under the Tile aren't empty.
    assert_eq!(mapped.cells.len(), 12);
    let empty: Vec<_> = mapped
        .cells
        .iter()
        .filter(|c| !c.empty)
        .map(|c| (c.row, c.col))
        .collect();
    assert_eq!(empty, [(0, 1), (0, 2)]);
    assert_eq!(mapped.cells[0].name, "Add Tile, row 1, column 1");
}

#[test]
fn save_states_conflicts_and_failures_read_as_words() {
    let mut shown = view(vec![grid("main", "Main", Vec::new())]);
    for (save, text, caution) in [
        (SaveState::None, "", false),
        (SaveState::Saved, "Sent", false),
        (SaveState::Unsent, "Unsent changes", false),
        (SaveState::Unconfirmed, "Couldn't confirm", true),
    ] {
        shown.save_state = save;
        let state = map(&shown, SCREEN, "Desk", false).state;
        assert_eq!(state.save_state, text);
        assert_eq!(state.save_caution, caution);
        assert_eq!(state.progress, -1);
    }

    shown.conflict = Some(ConflictView {
        name: "Elsewhere".into(),
    });
    let state = map(&shown, SCREEN, "desk", false).state;
    assert!(state.conflict);
    assert_eq!(state.conflict_title, "Desk has \u{201c}Elsewhere\u{201d}");

    // People never see the error enum's own text.
    shown.conflict = None;
    shown.outcome = Some(failed(
        CommandKind::Send,
        SessionError::Editor(EditorError::Rejected),
        "unresolved rejected fields",
    ));
    let state = map(&shown, SCREEN, "Desk", false).state;
    assert_eq!(
        state.notice,
        "Fix the Tiles marked with a caution sign, then send again."
    );
    assert_eq!(state.notice_detail, "unresolved rejected fields");

    shown.outcome = Some(failed(
        CommandKind::Edit,
        SessionError::Editor(EditorError::Edit(EditError::NoSpace)),
        "NoSpace",
    ));
    assert_eq!(
        map(&shown, SCREEN, "Desk", false).state.notice,
        "There's no room for that on this Page. Nothing changed."
    );
    // A transfer that failed keeps the Session's Device copy.
    let transfer = failed(
        CommandKind::Send,
        SessionError::Send("Companion stopped".into()),
        "Couldn't send to Desk. Check that it's plugged in, then retry.",
    );
    let failure = transfer.result.clone().unwrap_err();
    assert_eq!(
        failure_text(&failure, CommandKind::Send, "Desk"),
        "Couldn't send to Desk. Check that it's plugged in, then retry."
    );
    assert_eq!(
        problem_text(&too_long("t")),
        "Tile labels can be up to 16 characters."
    );
}

#[test]
fn a_view_without_a_layout_says_why() {
    let mut shown = view(Vec::new());
    shown.layout = None;
    shown.page = None;
    assert_eq!(map(&shown, SCREEN, "desk", true).state.state, "reading");
    shown.outcome = Some(failed(
        CommandKind::Load,
        SessionError::Layout(td_companion::layouts::LayoutError::NotFound),
        "",
    ));
    let state = map(&shown, SCREEN, "desk", false).state;
    assert_eq!(state.state, "empty");
    assert_eq!(state.message, "Desk has no Layout yet.");
    shown.outcome = Some(failed(
        CommandKind::Load,
        SessionError::Send("Companion stopped".into()),
        "",
    ));
    let state = map(&shown, SCREEN, "desk", false).state;
    assert_eq!(state.state, "unreadable");
    assert_eq!(state.message, "Couldn't read the Layout from desk.");
    // The raw error stays under Details, out of the body text.
    assert!(
        !state.detail.contains("Companion stopped"),
        "{}",
        state.detail
    );
    assert!(state.notice_detail.contains("Companion stopped"));
    shown.open = OpenState::Opening;
    assert_eq!(map(&shown, SCREEN, "desk", false).state.state, "opening");
}

#[test]
fn a_drag_lands_on_free_cells_inside_the_grid() {
    let mut shown = view(vec![grid(
        "main",
        "Main",
        vec![tile("a", "A", 0, 0, 1, 1), tile("b", "B", 0, 2, 1, 1)],
    )]);
    shown.tile = Some("a".into());
    let centre = |row: u8, col: u8| {
        let r = cell_rect(SCREEN, false, [3, 4], [row, col, 1, 1]).unwrap();
        (
            (r.x as f32 + f32::from(r.w) / 2.0) / 1024.0,
            (r.y as f32 + f32::from(r.h) / 2.0) / 600.0,
        )
    };
    let free = drop_target(&shown, SCREEN, "a", centre(0, 0), centre(2, 1), false);
    assert!(free.ok);
    assert_eq!(
        (free.row, free.col, free.row_span, free.col_span),
        (2, 1, 1, 1)
    );
    let taken = drop_target(&shown, SCREEN, "a", centre(0, 0), centre(0, 2), false);
    assert!(!taken.ok, "B is there");
    // The corner handle reaches B's column, so that span doesn't fit; one short of it does.
    assert!(!drop_target(&shown, SCREEN, "a", centre(0, 0), centre(1, 2), true).ok);
    let wide = drop_target(&shown, SCREEN, "a", centre(0, 0), centre(1, 1), true);
    assert!(wide.ok);
    assert_eq!((wide.row_span, wide.col_span), (2, 2));
    // Off the screen's grid.
    assert!(!drop_target(&shown, SCREEN, "a", centre(0, 0), (0.99, 0.99), false).ok);
}

/// Opens the editor on the first mock Device and waits until it settles.
fn open_editor(rig: &Rig) {
    rig.app.ui.set_page(Page::Layout);
    rig.app.ui.invoke_editor_shown();
    rig.until("the editor to read the Device", |r| {
        let state = r.app.ui.get_editor().state;
        state == "empty" || state == "ready"
    });
}

/// A blank Layout: one 3 × 4 Page.
fn start_blank(rig: &Rig) {
    open_editor(rig);
    rig.press("Start blank");
    rig.until("the blank Layout", |r| {
        r.app.ui.get_editor().state == "ready"
    });
}

fn worker(rig: &Rig) -> td_companion::editor::Worker {
    rig.screens.editor_workers().pop().expect("an editor")
}

fn tiles(rig: &Rig) -> Vec<crate::CanvasTile> {
    use slint::Model;
    rig.app.ui.get_editor_tiles().iter().collect()
}

#[test]
fn the_editor_opens_on_the_last_device_edited() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    rig.plug("mock0002", BoardKind::Device);
    open_editor(&rig);
    assert_eq!(rig.app.ui.get_editor().device_id, "mock0001");

    rig.one("Device", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the Device menu", |r| r.has("Mock Device mock0002"));
    rig.one("Mock Device mock0002", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the second Device", |r| {
        let state = r.app.ui.get_editor();
        state.device_id == "mock0002" && state.state == "empty"
    });
    let path = rig.dir.path().join("data");
    rig.until("the choice to be saved", |_| {
        Prefs::load(&path).value.editor_device.as_deref() == Some("mock0002")
    });
}

#[test]
fn add_page_is_disabled_at_eight_pages() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    for pages in 2..=MAX_PAGES {
        rig.press("Add Page");
        rig.until("the new Page", |r| {
            r.app.ui.get_editor().page_count == pages as i32
        });
    }
    let add = rig.one("Add Page", AccessibleRole::Button);
    assert_eq!(add.accessible_enabled(), Some(false));
    assert_eq!(
        add.accessible_description().unwrap(),
        "A Layout holds up to 8 Pages."
    );
    // Adding selects the new Page, so the eighth is current.
    assert_eq!(rig.app.ui.get_editor().page_index, 7);
    add.invoke_accessible_default_action();
    settle(&rig.app);
    std::thread::sleep(Duration::from_millis(100));
    rig.until("no ninth Page", |r| r.app.ui.get_editor().page_count == 8);

    // Remove Page frees a slot.
    rig.one("More for Page 8", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the entry menu", |r| r.has("Remove Page"));
    rig.press("Remove Page");
    rig.until("seven Pages", |r| r.app.ui.get_editor().page_count == 7);
    assert_eq!(
        rig.one("Add Page", AccessibleRole::Button)
            .accessible_enabled(),
        Some(true)
    );
}

#[test]
fn the_fold_preference_is_kept_across_restarts() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    rig.one("Pages", AccessibleRole::List);
    rig.press("Hide Pages");
    rig.until("the filmstrip to fold", |r| !r.has("Pages"));
    let data = rig.dir.path().join("data");
    rig.until("the choice to be saved", |_| {
        Prefs::load(&data).value.filmstrip_folded
    });

    // A new window over the same data folder opens folded.
    let again = App::new(Options {
        data_dir: data.clone(),
        config: td_core::app_dirs::Dirs {
            config: rig.dir.path().join("config"),
        },
        close_to_tray: false,
    })
    .unwrap();
    assert!(again.state.filmstrip_folded());

    rig.press("Show Pages");
    rig.until("the filmstrip", |r| r.has("Pages"));
    rig.until("the choice to be saved", |_| {
        !Prefs::load(&data).value.filmstrip_folded
    });
}

#[test]
fn tiles_select_move_resize_and_go_with_the_keyboard() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    rig.one("Add Tile, row 1, column 1", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the new Tile, selected", |r| {
        r.app.ui.get_editor().tile_selected
    });
    let name = "Tile, row 1, column 1, 1 by 1";
    // A click selects it and puts the keyboard on the canvas.
    rig.one(name, AccessibleRole::Button)
        .mock_single_click(slint::platform::PointerEventButton::Left);
    let key = |key: Key, shift: bool| {
        let window = rig.app.ui.window();
        if shift {
            window.dispatch_event(WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            });
        }
        window.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
        if shift {
            window.dispatch_event(WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            });
        }
    };
    key(Key::RightArrow, false);
    rig.until("the move", |r| r.has("Tile, row 1, column 2, 1 by 1"));
    key(Key::DownArrow, true);
    rig.until("the resize", |r| r.has("Tile, row 1, column 2, 2 by 1"));
    // At the grid's edge an arrow does nothing.
    key(Key::UpArrow, false);
    settle(&rig.app);
    rig.until("still in row 1", |r| r.has("Tile, row 1, column 2, 2 by 1"));
    key(Key::Escape, false);
    rig.until("Page mode", |r| !r.app.ui.get_editor().tile_selected);
    rig.one("Tile, row 1, column 2, 2 by 1", AccessibleRole::Button)
        .mock_single_click(slint::platform::PointerEventButton::Left);
    rig.until("Tile mode", |r| r.app.ui.get_editor().tile_selected);
    key(Key::UpArrow, true);
    rig.until("the shrink", |r| r.has("Tile, row 1, column 2, 1 by 1"));

    // A drag moves it to the free cell it's dropped on.
    let tile = rig.one("Tile, row 1, column 2, 1 by 1", AccessibleRole::Button);
    let target = rig.one("Add Tile, row 3, column 4", AccessibleRole::Button);
    let corner = target.absolute_position();
    let size = target.size();
    tile.mock_drag(
        slint::LogicalPosition::new(corner.x + size.width / 2.0, corner.y + size.height / 2.0),
        slint::platform::PointerEventButton::Left,
    );
    rig.until("the drop", |r| r.has("Tile, row 3, column 4, 1 by 1"));

    // Delete removes the selected Tile.
    let tile = rig.one(&tiles(&rig)[0].name, AccessibleRole::Button);
    tile.mock_single_click(slint::platform::PointerEventButton::Left);
    rig.until("Tile mode", |r| r.app.ui.get_editor().tile_selected);
    key(Key::Delete, false);
    rig.until("no Tiles", |r| tiles(r).is_empty());
}

#[test]
fn send_shows_the_save_state_and_a_refused_label_blocks_it() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    rig.until("Unsent changes", |r| {
        r.app.ui.get_editor().save_state == "Unsent changes"
    });
    rig.press("Send to Device");
    rig.until("Sent", |r| r.app.ui.get_editor().save_state == "Sent");
    assert!(
        rig.deck("mock0001")
            .layout(Orientation::Landscape)
            .is_some()
    );

    rig.one("Add Tile, row 2, column 2", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the Tile", |r| tiles(r).len() == 1);
    let id = tiles(&rig)[0].id.to_string();
    // The Label field arrives with the toolbar; the Worker takes the same command.
    worker(&rig).dispatch(Command::Edit(Edit::Label {
        tile: id,
        label: "Seventeen letters".into(),
    }));
    rig.until("the caution mark", |r| {
        tiles(r)[0].problem == "Tile labels can be up to 16 characters."
    });
    rig.app.ui.invoke_editor_clear_selection();
    rig.until("Page mode", |r| !r.app.ui.get_editor().tile_selected);
    rig.press("Send to Device");
    rig.until("the refusal", |r| !r.app.ui.get_editor().notice.is_empty());
    let state = rig.app.ui.get_editor();
    assert_eq!(
        state.notice,
        "Fix the Tiles marked with a caution sign, then send again."
    );
    assert!(state.tile_selected, "Send selects the first problem Tile");
    assert_eq!(state.save_state, "Unsent changes");
    let tile = rig.one(
        "Seventeen letters, row 2, column 2, 1 by 1",
        AccessibleRole::Button,
    );
    assert_eq!(
        tile.accessible_description().unwrap(),
        "Tile labels can be up to 16 characters."
    );
    rig.one(
        "Page 1 of 1, Page 1, needs a fix, current",
        AccessibleRole::ListItem,
    );
}

fn elsewhere() -> Vec<u8> {
    td_core::Layout {
        schema: 1,
        name: "Elsewhere".into(),
        orientation: Orientation::Landscape,
        full_screen: false,
        pages: vec![td_core::layout::Page::Grid {
            id: "page1".into(),
            title: "Other".into(),
            rows: 2,
            cols: 2,
            tiles: vec![],
        }],
    }
    .to_canonical_json()
}

#[test]
fn keep_mine_after_a_refused_send_sends_again_and_replace_takes_the_device_layout() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    rig.press("Send to Device");
    rig.until("Sent", |r| r.app.ui.get_editor().save_state == "Sent");

    // Someone else writes the Device; the next send is refused and asks which stays.
    rig.deck("mock0001")
        .seed_layout(Orientation::Landscape, Some(elsewhere()));
    rig.one("Add Tile, row 1, column 1", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the Tile", |r| tiles(r).len() == 1);
    rig.press("Send to Device");
    let title = "Mock Device mock0001 has \u{201c}Elsewhere\u{201d}";
    rig.until("the conflict dialog", |r| r.has(title));
    rig.one(title, AccessibleRole::Groupbox);
    rig.press("Keep mine");
    rig.until("the second send", |r| {
        let state = r.app.ui.get_editor();
        !state.conflict && state.save_state == "Sent"
    });
    let stored = rig.deck("mock0001").layout(Orientation::Landscape).unwrap();
    assert!(
        String::from_utf8(stored).unwrap().contains("New Layout"),
        "mine reached the Device"
    );

    // Read from Device after another outside write; Replace takes the Device's Layout.
    rig.deck("mock0001")
        .seed_layout(Orientation::Landscape, Some(elsewhere()));
    rig.one("Layout", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the Layout menu", |r| r.has("Read from Device"));
    rig.press("Read from Device");
    rig.until("the conflict dialog", |r| r.has(title));
    rig.press("Replace");
    rig.until("the Device's Layout", |r| {
        let state = r.app.ui.get_editor();
        !state.conflict && state.page_title == "Other"
    });
    assert_eq!(rig.app.ui.get_editor().rows, 2);
}

#[test]
fn quitting_saves_an_edit_queued_behind_a_device_read() {
    let mut rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    rig.press("Send to Device");
    rig.until("Sent", |r| r.app.ui.get_editor().save_state == "Sent");

    // A slow read holds the Worker's queue; a new Tile waits behind it.
    let checkpoints = rig.handle().checkpoints().clone();
    checkpoints.hold(Checkpoint::BeforeEditorLoad);
    rig.app.ui.invoke_editor_read();
    rig.until("the held read", |_| {
        checkpoints.waiting(Checkpoint::BeforeEditorLoad)
    });
    rig.one("Add Tile, row 1, column 1", AccessibleRole::Button)
        .invoke_accessible_default_action();
    let view = worker(&rig).subscribe();
    // The event loop has ended; the read comes back while the window closes its editors.
    checkpoints.release(Checkpoint::BeforeEditorLoad, false);
    crate::close_editors(&rig.screens, rig.runtime.handle(), Duration::from_secs(10));
    // Before the Companion stops: the Session has dropped and its last save is on disk.
    assert!(view.has_changed().is_err(), "the editor has closed");
    let data = rig.dir.path().join("data");
    let saved = td_companion::editor::Editor::open(data, "mock0001").unwrap();
    let draft = saved.draft(Orientation::Landscape).unwrap();
    let td_core::layout::Page::Grid { tiles, .. } = &draft.layout().pages[0] else {
        panic!("a grid Page");
    };
    assert_eq!(tiles.len(), 1, "the queued Tile was saved");
    let companion = rig.companion.take().unwrap();
    rig.runtime.block_on(companion.shutdown());
}

#[test]
fn quitting_waits_for_an_editor_no_longer_than_its_limit() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    let checkpoints = rig.handle().checkpoints().clone();
    checkpoints.hold(Checkpoint::BeforeEditorLoad);
    rig.app.ui.invoke_editor_read();
    rig.until("the held read", |_| {
        checkpoints.waiting(Checkpoint::BeforeEditorLoad)
    });
    // The read never returns while the window quits.
    let started = Instant::now();
    crate::close_editors(
        &rig.screens,
        rig.runtime.handle(),
        Duration::from_millis(300),
    );
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(300) && waited < Duration::from_secs(3),
        "waited {waited:?}"
    );
    checkpoints.release(Checkpoint::BeforeEditorLoad, false);
}

#[test]
fn at_the_largest_text_size_the_editor_fits_the_narrowest_window() {
    let rig = Rig::start(500.0);
    rig.app.ui.window().set_size(LogicalSize::new(500.0, 900.0));
    look::apply(
        &rig.app.ui,
        &look::Look {
            text_scale: 2.25,
            ..Default::default()
        },
    );
    rig.plug("mock0001", BoardKind::Device);
    rig.plug("mock0002", BoardKind::Device);
    open_editor(&rig);
    assert_fits(&rig, 500.0, "the empty editor");
    rig.press("Start a Layout");
    rig.until("the starter Layout", |r| {
        r.app.ui.get_editor().state == "ready"
    });
    // Below 720 px the filmstrip folds by itself.
    assert!(!rig.has("Pages"));
    assert_fits(&rig, 500.0, "the editor, folded");
    rig.press("Show Pages");
    // The testing backend's clock only moves when told to; let the 200 ms unfold finish.
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(250));
    rig.until("the filmstrip", |r| r.has("Pages"));
    assert_fits(&rig, 500.0, "the editor with its filmstrip");
    rig.one("Device", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the Device menu", |r| r.has("Mock Device mock0002"));
    assert_fits(&rig, 500.0, "the Device menu");
}

#[test]
fn the_corner_handle_follows_the_pointer_one_to_one() {
    use slint::{LogicalPosition, platform::PointerEventButton};
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    start_blank(&rig);
    rig.one("Add Tile, row 1, column 1", AccessibleRole::Button)
        .invoke_accessible_default_action();
    rig.until("the new Tile, selected", |r| {
        r.app.ui.get_editor().tile_selected
    });
    let tile = rig.one("Tile, row 1, column 1, 1 by 1", AccessibleRole::Button);
    let (at, size) = (tile.absolute_position(), tile.size());
    // The handle's centre sits 2 px inside the Tile's bottom-right corner.
    let start = LogicalPosition::new(at.x + size.width - 2.0, at.y + size.height - 2.0);
    // One cell to the right: the distance between two neighbouring cells.
    let next = rig
        .one("Add Tile, row 1, column 2", AccessibleRole::Button)
        .absolute_position();
    let pitch = next.x - at.x;
    let window = rig.app.ui.window();
    window.dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    for step in 1..=10 {
        window.dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(start.x + pitch * step as f32 / 10.0, start.y),
        });
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: LogicalPosition::new(start.x + pitch, start.y),
        button: PointerEventButton::Left,
    });
    rig.until("the resize", |r| {
        tiles(r).first().is_some_and(|t| t.w > 0.3)
            || r.has("Tile, row 1, column 1, 1 by 2")
            || r.has("Tile, row 1, column 1, 1 by 3")
    });
    assert!(
        rig.has("Tile, row 1, column 1, 1 by 2"),
        "one cell of pointer travel is one more column: {:?}",
        tiles(&rig)
            .iter()
            .map(|t| t.name.to_string())
            .collect::<Vec<_>>()
    );
}
