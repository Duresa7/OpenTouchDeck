//! Headless tests of the window on Slint's testing backend: no window opens and no display is
//! needed. They find elements the way a screen reader does, by accessible role and label.
use crate::{
    App, AppWindow, ContrastColors, Options, Page, Theme, about, home,
    home::tests::{connected, device},
    look,
    prefs::{Placement, Prefs},
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use slint::{
    ComponentHandle, LogicalSize,
    platform::{PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, path::Path, rc::Rc};
use td_companion::{
    companion::Change,
    devices::{DeviceUpdated, LinkState},
};

fn init() {
    thread_local!(static READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    READY.with(|ready| {
        if !ready.replace(true) {
            i_slint_backend_testing::init_no_event_loop();
        }
    });
}

fn app(dir: &Path, close_to_tray: bool) -> App {
    init();
    App::new(Options {
        data_dir: dir.to_owned(),
        close_to_tray,
    })
    .unwrap()
}

/// Slint creates the contents of repeaters and conditionals lazily, before it renders or
/// handles input. The testing backend never renders, so a pointer move outside the window
/// stands in for the next frame. The window has to be shown.
fn settle(ui: &AppWindow) {
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(-1.0, -1.0),
    });
}

fn labelled(ui: &AppWindow, label: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(ui, label).collect()
}

/// The one element with this label.
fn one(ui: &AppWindow, label: &str) -> ElementHandle {
    let found = labelled(ui, label);
    assert_eq!(found.len(), 1, "elements labelled {label:?}");
    found.into_iter().next().unwrap()
}

fn with_role(ui: &AppWindow, label: &str, role: AccessibleRole) -> ElementHandle {
    let found: Vec<_> = labelled(ui, label)
        .into_iter()
        .filter(|element| element.accessible_role() == Some(role))
        .collect();
    assert_eq!(found.len(), 1, "{role:?} elements labelled {label:?}");
    found.into_iter().next().unwrap()
}

fn text_containing(ui: &AppWindow, needle: &str) -> bool {
    let needle = needle.to_owned();
    !ui.root_element()
        .query_descendants()
        .match_predicate(move |element| {
            element
                .accessible_label()
                .is_some_and(|label| label.contains(&needle))
        })
        .find_all()
        .is_empty()
}

#[test]
fn home_with_no_devices_asks_for_one() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    home::show(&app.ui, &[]);
    assert_eq!(app.ui.get_page(), Page::Home);
    one(&app.ui, "Plug in your Device");
    assert!(text_containing(&app.ui, "with a USB data cable"));
    assert!(labelled(&app.ui, "Devices").is_empty(), "no empty list");
}

#[test]
fn home_lists_a_connected_device() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    home::show(&app.ui, &[device("a1", Some("Desk Deck"), connected())]);

    let list = with_role(&app.ui, "Devices", AccessibleRole::List);
    assert_eq!(list.accessible_item_count(), Some(1));
    let card = with_role(&app.ui, "Desk Deck", AccessibleRole::ListItem);
    assert_eq!(card.accessible_description().unwrap(), "Connected over USB");
    assert!(labelled(&app.ui, "Plug in your Device").is_empty());
}

#[test]
fn home_lists_a_disconnected_device() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    home::show(&app.ui, &[device("b2", Some("Studio"), LinkState::Offline)]);
    let card = with_role(&app.ui, "Studio", AccessibleRole::ListItem);
    assert_eq!(card.accessible_description().unwrap(), "Not connected");
}

#[test]
fn home_follows_a_device_as_it_disconnects() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    home::show(&app.ui, &[device("a1", Some("Desk Deck"), connected())]);
    home::update(
        &app.ui,
        &Change::DeviceUpdated(DeviceUpdated(device(
            "a1",
            Some("Desk Deck"),
            LinkState::Disconnected,
        ))),
    );
    let card = with_role(&app.ui, "Desk Deck", AccessibleRole::ListItem);
    assert_eq!(card.accessible_description().unwrap(), "Not connected");
}

#[test]
fn the_navigation_pane_opens_about_and_home() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    let about = with_role(&app.ui, "About", AccessibleRole::Tab);
    let home_tab = with_role(&app.ui, "Home", AccessibleRole::Tab);
    assert_eq!(home_tab.accessible_item_selected(), Some(true));
    assert_eq!(about.accessible_item_selected(), Some(false));

    about.invoke_accessible_default_action();
    assert_eq!(app.ui.get_page(), Page::About);
    assert_eq!(about.accessible_item_selected(), Some(true));
    with_role(&app.ui, "About", AccessibleRole::TabPanel);

    home_tab.invoke_accessible_default_action();
    assert_eq!(app.ui.get_page(), Page::Home);
}

#[test]
fn one_click_opens_a_page_or_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let record = opened.clone();
    app.ui
        .on_open_url(move |url| record.borrow_mut().push(url.into()));
    app.ui.show().unwrap();
    settle(&app.ui);

    with_role(&app.ui, "About", AccessibleRole::Tab).mock_single_click(PointerEventButton::Left);
    assert_eq!(app.ui.get_page(), Page::About);
    settle(&app.ui);
    with_role(&app.ui, "Get the source code", AccessibleRole::Button)
        .mock_single_click(PointerEventButton::Left);
    assert_eq!(
        *opened.borrow(),
        ["https://github.com/Duresa7/OpenTouchDeck"]
    );
    with_role(&app.ui, "Home", AccessibleRole::Tab).mock_single_click(PointerEventButton::Left);
    assert_eq!(app.ui.get_page(), Page::Home);
}

#[test]
fn about_shows_the_version_licence_slint_credit_and_components() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let record = opened.clone();
    app.ui
        .on_open_url(move |url| record.borrow_mut().push(url.into()));
    app.ui.set_page(Page::About);
    // Like rendering, the search skips what is scrolled out of view, so show the whole page.
    app.ui.window().set_size(LogicalSize::new(800.0, 6000.0));
    app.ui.show().unwrap();
    settle(&app.ui);

    one(&app.ui, &format!("Version {}", env!("CARGO_PKG_VERSION")));
    one(&app.ui, "Copyright Duresa Kadi");
    assert!(text_containing(
        &app.ui,
        "GNU General Public License, version 3 (GPL-3.0)"
    ));
    assert!(text_containing(&app.ui, "no warranty"));
    assert!(text_containing(&app.ui, "Apache License 2.0"));
    assert!(text_containing(&app.ui, "built with Slint"));

    for link in [
        "Read the GNU General Public License",
        "Get the source code",
        "slint.dev",
    ] {
        with_role(&app.ui, link, AccessibleRole::Button).invoke_accessible_default_action();
    }
    assert_eq!(
        *opened.borrow(),
        [
            "https://www.gnu.org/licenses/gpl-3.0.html",
            "https://github.com/Duresa7/OpenTouchDeck",
            "https://slint.dev",
        ]
    );

    let list = with_role(&app.ui, "Components", AccessibleRole::List);
    assert_eq!(list.accessible_item_count(), Some(about::COMPONENTS.len()));
    let slint = with_role(&app.ui, "slint", AccessibleRole::ListItem);
    assert_eq!(slint.accessible_description().unwrap(), "GPL-3.0-only");
    let tokio = with_role(&app.ui, "tokio", AccessibleRole::ListItem);
    assert_eq!(tokio.accessible_description().unwrap(), "MIT");
}

#[test]
fn the_first_close_with_a_tray_explains_once_then_hides() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), true);
    app.ui.show().unwrap();

    app.ui.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        app.ui.window().is_visible(),
        "the notice keeps the window open"
    );
    assert!(app.ui.get_close_notice_open());
    with_role(
        &app.ui,
        "Open TouchDeck keeps running",
        AccessibleRole::Groupbox,
    );
    assert!(!Prefs::load(dir.path()).value.close_notice_shown);

    with_role(&app.ui, "Keep running", AccessibleRole::Button).invoke_accessible_default_action();
    assert!(!app.ui.get_close_notice_open());
    assert!(!app.ui.window().is_visible());
    assert!(Prefs::load(dir.path()).value.close_notice_shown);
    assert!(!app.state.quit_requested());

    // From now on closing hides the window straight away.
    app.ui.show().unwrap();
    app.ui.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(!app.ui.get_close_notice_open());
    assert!(!app.ui.window().is_visible());
    assert!(!app.state.quit_requested());
}

#[test]
fn the_notice_is_not_shown_again_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut prefs = Prefs::load(dir.path());
    prefs.value.close_notice_shown = true;
    prefs.save().unwrap();

    let app = app(dir.path(), true);
    app.ui.show().unwrap();
    app.ui.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(!app.ui.get_close_notice_open());
    assert!(!app.ui.window().is_visible());
}

#[test]
fn quit_in_the_close_notice_ends_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), true);
    app.ui.show().unwrap();
    app.ui.window().dispatch_event(WindowEvent::CloseRequested);
    with_role(&app.ui, "Quit", AccessibleRole::Button).invoke_accessible_default_action();
    assert!(app.state.quit_requested());
    assert!(
        Prefs::load(dir.path()).value.close_notice_shown,
        "seen counts"
    );
}

#[test]
fn closing_without_a_tray_ends_the_program_and_saves_the_place() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    app.ui.show().unwrap();
    app.ui.window().set_size(LogicalSize::new(820.0, 610.0));

    app.ui.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(app.state.quit_requested());
    assert!(
        !app.ui.get_close_notice_open(),
        "no tray, so nothing to explain"
    );
    let prefs = Prefs::load(dir.path()).value;
    let placement = prefs.placement.expect("placement saved");
    assert_eq!((placement.width, placement.height), (820.0, 610.0));
    assert!(!placement.maximized);
    assert!(!prefs.close_notice_shown);
}

#[test]
fn the_window_opens_at_its_saved_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut prefs = Prefs::load(dir.path());
    prefs.value.placement = Some(Placement {
        x: 40,
        y: 30,
        width: 700.0,
        height: 520.0,
        maximized: false,
    });
    prefs.save().unwrap();

    let app = app(dir.path(), false);
    crate::window::restore(&app.ui, &app.state);
    let size = app
        .ui
        .window()
        .size()
        .to_logical(app.ui.window().scale_factor());
    assert_eq!((size.width, size.height), (700.0, 520.0));
}

/// Every control, measured with real fonts, ends inside the window.
fn assert_controls_fit(ui: &AppWindow, width: f32, page: &str) {
    for role in [AccessibleRole::Button, AccessibleRole::Tab] {
        for element in ui
            .root_element()
            .query_descendants()
            .match_accessible_role(role)
            .find_all()
        {
            let left = element.absolute_position().x;
            let right = left + element.size().width;
            let label = element.accessible_label().unwrap_or_default();
            assert!(
                left >= 0.0 && right <= width + 0.5,
                "{page}: {label:?} spans {left}..{right} in a {width} px window"
            );
            assert!(element.size().height > 0.0, "{page}: {label:?} is visible");
        }
    }
}

#[test]
fn at_the_largest_text_size_every_control_fits_the_narrowest_window() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), true);
    look::apply(
        &app.ui,
        &look::Look {
            text_scale: 2.25,
            ..Default::default()
        },
    );
    assert_eq!(app.ui.global::<Theme>().get_body(), 14.0 * 2.25);
    app.ui.window().set_size(LogicalSize::new(500.0, 360.0));
    app.ui.show().unwrap();
    home::show(&app.ui, &[device("a1", Some("Desk Deck"), connected())]);
    assert_controls_fit(&app.ui, 500.0, "Home");

    app.ui.set_page(Page::About);
    assert_controls_fit(&app.ui, 500.0, "About");

    app.ui.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(app.ui.get_close_notice_open());
    assert_controls_fit(&app.ui, 500.0, "close notice");
}

#[test]
fn a_contrast_theme_replaces_every_colour_with_the_system_ones() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false);
    let colour = slint::Color::from_rgb_u8;
    let system = ContrastColors {
        window: colour(0x20, 0x20, 0x20),
        window_text: colour(0xff, 0xff, 0xff),
        highlight: colour(0x8e, 0xe3, 0xf0),
        highlight_text: colour(0x26, 0x3b, 0x50),
        hotlight: colour(0x75, 0xe9, 0xfc),
        gray_text: colour(0xa6, 0xa6, 0xa6),
        button_face: colour(0x20, 0x20, 0x20),
        button_text: colour(0xff, 0xff, 0xff),
    };
    look::apply(
        &app.ui,
        &look::Look {
            text_scale: 1.0,
            high_contrast: true,
            contrast: Some(system.clone()),
        },
    );
    let theme = app.ui.global::<Theme>();
    let brush = |c: slint::Color| slint::Brush::from(c);
    assert_eq!(theme.get_window_background(), brush(system.window));
    assert_eq!(theme.get_text(), brush(system.window_text));
    assert_eq!(theme.get_secondary_text(), brush(system.window_text));
    assert_eq!(theme.get_accent(), brush(system.highlight));
    assert_eq!(theme.get_link(), brush(system.hotlight));
    assert_eq!(theme.get_control(), brush(system.button_face));
    assert_eq!(
        theme.get_good(),
        brush(system.window_text),
        "words carry the state"
    );

    // Without system colours (Linux), black and white stand in.
    look::apply(
        &app.ui,
        &look::Look {
            text_scale: 1.0,
            high_contrast: true,
            contrast: None,
        },
    );
    let text = theme.get_text().color();
    let background = theme.get_window_background().color();
    assert_ne!(text, background);
    assert!([text, background].contains(&colour(0, 0, 0)));
    assert!([text, background].contains(&colour(0xff, 0xff, 0xff)));
}
