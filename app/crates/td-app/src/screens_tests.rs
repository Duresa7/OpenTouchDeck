//! Setup and Settings, headless, against the in-process Companion and mock Devices. They act the
//! way a screen reader does: find controls by role and label and invoke them.
use crate::{
    App, BoardCard, DevicePage, Options, Page, PairedApp, Theme, Wizard, look, prefs::Prefs,
    screens::Screens,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use slint::{
    ComponentHandle, LogicalSize, SharedString,
    platform::{Key, WindowEvent},
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
use td_companion::{
    companion::{Companion, Config},
    devices::{BoardKind, DeviceId, LinkState, MockPorts, SetupState},
    features::firmware::ConsentAnswer,
};

struct Rig {
    runtime: tokio::runtime::Runtime,
    companion: Option<Companion>,
    ports: MockPorts,
    app: App,
    screens: Rc<Screens>,
    dir: tempfile::TempDir,
}

impl Rig {
    /// A Companion on mock ports, the window wired to it and shown at `width` × 2000 px so the
    /// whole page can be searched.
    fn start(width: f32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let mut config = Config::mock(dir.path().join("data"));
        config.devices = td_companion::devices::Config::mock(None);
        let companion = runtime.block_on(Companion::start(config)).unwrap();
        let handle = companion.handle();
        let ports = handle.devices().mock().unwrap().0.clone();
        handle.firmware().script(vec![ConsentAnswer::Accept; 16]);
        crate::ui_tests::init();
        let app = App::new(Options {
            data_dir: dir.path().join("data"),
            config: td_core::app_dirs::Dirs {
                config: dir.path().join("config"),
            },
            close_to_tray: false,
        })
        .unwrap();
        app.ui.window().set_size(LogicalSize::new(width, 2000.0));
        app.ui.show().unwrap();
        let screens = app.connect(&handle, runtime.handle());
        Self {
            runtime,
            companion: Some(companion),
            ports,
            app,
            screens,
            dir,
        }
    }

    fn handle(&self) -> td_companion::companion::Handle {
        self.companion.as_ref().unwrap().handle()
    }

    /// Refreshes the window until `done` holds, as the UI thread would on each change.
    fn until(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let started = Instant::now();
        loop {
            self.screens.refresh();
            settle(&self.app);
            if done(self) {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn plug(&self, serial: &str, kind: BoardKind) -> DeviceId {
        self.ports.plug(serial, kind);
        let id = DeviceId::parse(serial).unwrap();
        let handle = self.handle();
        self.until("the Device to connect", |_| {
            handle
                .devices()
                .get(&id)
                .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
        });
        id
    }

    fn deck(&self, serial: &str) -> td_link::MockDeck {
        self.ports.deck(serial).unwrap()
    }

    fn labelled(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.app.ui, label).collect()
    }

    fn has(&self, label: &str) -> bool {
        !self.labelled(label).is_empty()
    }

    fn one(&self, label: &str, role: AccessibleRole) -> ElementHandle {
        let found: Vec<_> = self
            .labelled(label)
            .into_iter()
            .filter(|e| e.accessible_role() == Some(role))
            .collect();
        assert_eq!(found.len(), 1, "{role:?} elements labelled {label:?}");
        found.into_iter().next().unwrap()
    }

    fn press(&self, label: &str) {
        self.one(label, AccessibleRole::Button)
            .invoke_accessible_default_action();
    }

    fn text_containing(&self, needle: &str) -> bool {
        let needle = needle.to_owned();
        !self
            .app
            .ui
            .root_element()
            .query_descendants()
            .match_predicate(move |e| e.accessible_label().is_some_and(|l| l.contains(&needle)))
            .find_all()
            .is_empty()
    }

    /// Types into a text field and presses Enter, as a person would.
    fn type_and_enter(&self, label: &str, text: &str) {
        let field = self.one(label, AccessibleRole::TextInput);
        field.set_accessible_value(text);
        field.mock_single_click(slint::platform::PointerEventButton::Left);
        let window = self.app.ui.window();
        window.dispatch_event(WindowEvent::KeyPressed {
            text: Key::Return.into(),
        });
        window.dispatch_event(WindowEvent::KeyReleased {
            text: Key::Return.into(),
        });
    }

    fn open_device_settings(&self, name: &str) {
        self.app.ui.set_page(Page::Settings);
        self.until("the Device in Settings", |r| r.has(name));
        self.one(name, AccessibleRole::Button)
            .invoke_accessible_default_action();
        self.until("its settings to load", |r| {
            r.app.ui.get_device_page().editable
        });
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(companion) = self.companion.take() {
            self.runtime.block_on(companion.shutdown());
        }
    }
}

fn settle(app: &App) {
    app.ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(-1.0, -1.0),
    });
}

#[test]
fn a_new_board_installs_and_setup_names_and_turns_it() {
    let rig = Rig::start(960.0);
    rig.ports.plug("mock0009", BoardKind::Silent);
    rig.app.ui.set_page(Page::Setup);
    rig.until("the New board card", |r| r.has("New board found"));
    assert!(rig.text_containing("doesn't run Open TouchDeck Firmware yet"));
    assert_eq!(rig.app.ui.get_setup_count(), 1);

    rig.press("Install Firmware");
    // Setup opens by itself once the board is a Device.
    rig.until("setup to open", |r| r.app.ui.get_wizard_open());
    assert_eq!(rig.app.ui.get_page(), Page::Setup);
    rig.one("Set up Open TouchDeck", AccessibleRole::Groupbox);
    assert!(rig.has("Step 1 of 3"));

    // A blank name is refused here, with the reason.
    rig.type_and_enter("Name", "   ");
    rig.until("the name problem", |r| r.app.ui.get_wizard().name_invalid);
    assert_eq!(rig.app.ui.get_wizard().name_hint, "Enter a name.");

    rig.type_and_enter("Name", "Desk");
    rig.until("the orientation step", |r| r.has("Step 2 of 3"));
    let id = DeviceId::parse("mock0009").unwrap();
    let device = rig.handle().devices().get(&id).unwrap();
    assert_eq!(device.name.as_deref(), Some("Desk"));
    assert_eq!(device.setup, SetupState::Ready);

    // The chosen orientation has focus, and the arrow keys move the choice.
    for (event, key) in [(true, Key::DownArrow), (false, Key::DownArrow)] {
        let text: SharedString = key.into();
        rig.app.ui.window().dispatch_event(if event {
            WindowEvent::KeyPressed { text }
        } else {
            WindowEvent::KeyReleased { text }
        });
    }
    rig.until("Portrait chosen", |r| {
        r.one("Portrait", AccessibleRole::RadioButton)
            .accessible_checked()
            == Some(true)
    });
    // Focus followed the choice: Down wraps round to Landscape, Up comes back to Portrait.
    for (key, expected) in [(Key::DownArrow, "Landscape"), (Key::UpArrow, "Portrait")] {
        let text: SharedString = key.into();
        let window = rig.app.ui.window();
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
        rig.until(expected, |r| {
            r.one(expected, AccessibleRole::RadioButton)
                .accessible_checked()
                == Some(true)
        });
    }
    rig.press("Next");
    rig.until("the last step", |r| r.has("Desk is ready"));
    assert_eq!(
        rig.deck("mock0009").settings().orientation,
        td_core::Orientation::Portrait
    );
    rig.press("Done");
    rig.until("setup to close", |r| !r.app.ui.get_wizard_open());
    assert!(rig.has("Nothing to set up"));
}

#[test]
fn closing_setup_skips_it_until_it_is_opened_again() {
    let rig = Rig::start(960.0);
    rig.ports.plug("mock0009", BoardKind::Silent);
    rig.app.ui.set_page(Page::Setup);
    rig.until("the New board card", |r| r.has("Install Firmware"));
    rig.press("Install Firmware");
    rig.until("setup to open", |r| r.app.ui.get_wizard_open());

    rig.press("Close setup");
    rig.until("setup to close", |r| !r.app.ui.get_wizard_open());
    // It doesn't open by itself again, also after a restart of the app.
    for _ in 0..5 {
        rig.screens.refresh();
    }
    assert!(!rig.app.ui.get_wizard_open());
    let skipped = std::fs::read_to_string(rig.dir.path().join("data/setup-skipped.json")).unwrap();
    assert!(skipped.contains("mock0009"));

    rig.press("Set up Open TouchDeck");
    rig.until("setup to open again", |r| r.app.ui.get_wizard_open());
    assert!(rig.has("Step 1 of 3"));
}

/// Waits for the Companion without refreshing the window, so changes pile up as they do when
/// the UI thread is busy and their wake-ups coalesce.
fn without_refresh(what: &str, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_reinstall_asks_for_setup_again_even_when_its_changes_arrive_together() {
    let rig = Rig::start(960.0);
    let id = DeviceId::parse("mock0009").unwrap();
    // Setup was closed for this board before.
    rig.handle().setup_skips().skip(&id);
    rig.ports.plug("mock0009", BoardKind::Silent);
    rig.app.ui.set_page(Page::Setup);
    rig.until("the New board card", |r| r.has("Install Firmware"));
    rig.press("Install Firmware");
    // The window sees nothing until the board is a Device: the install's "done" and the new
    // Device arrive in one refresh, and the board's card is already gone.
    let handle = rig.handle();
    without_refresh("the new Device", || {
        handle.devices().get(&id).is_some_and(|d| {
            d.setup == SetupState::NeedsSetup && matches!(d.link, LinkState::Connected { .. })
        })
    });
    rig.screens.refresh();
    settle(&rig.app);
    assert!(
        rig.app.ui.get_wizard_open(),
        "setup opens for the reinstalled board"
    );
    assert!(!rig.handle().setup_skips().skipped().contains("mock0009"));
}

#[test]
fn an_ignored_board_waits_in_its_own_list() {
    let rig = Rig::start(960.0);
    rig.ports.plug("mock0009", BoardKind::Silent);
    rig.app.ui.set_page(Page::Setup);
    rig.until("the New board card", |r| r.has("Ignore"));
    rig.press("Ignore");
    rig.until("the ignored list", |r| r.has("New board on mock0009"));
    assert!(!rig.has("Install Firmware"));
    rig.press("New board on mock0009");
    rig.until("the card again", |r| r.has("Install Firmware"));
}

#[test]
fn a_rename_saves_or_rolls_back_to_what_the_device_keeps() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    rig.open_device_settings("Mock Device mock0001");
    let name = rig.one("Name", AccessibleRole::TextInput);
    assert_eq!(name.accessible_value().unwrap(), "Mock Device mock0001");

    rig.type_and_enter("Name", "tab\there");
    rig.until("the name problem", |r| {
        r.app.ui.get_device_page().name_invalid
    });
    assert!(rig.text_containing("Remove tabs, line breaks and other special characters."));

    rig.type_and_enter("Name", "Desk left");
    rig.until("Saved", |r| r.app.ui.get_device_page().notice == "Saved");
    assert_eq!(rig.app.ui.get_device_page().name, "Desk left");

    // The Device stops answering: the change rolls back, and the notice says why.
    rig.deck("mock0001").delay_replies(Duration::from_secs(3));
    rig.type_and_enter("Name", "Too slow");
    rig.until("the failure", |r| {
        r.app.ui.get_device_page().notice_kind == "error"
    });
    let page = rig.app.ui.get_device_page();
    assert_eq!(
        page.notice,
        "Couldn't save to Desk left. Check that it's plugged in, then retry."
    );
    assert!(!page.notice_detail.is_empty(), "Details");
    assert_eq!(page.name_value, "Desk left");
    assert_eq!(
        rig.one("Name", AccessibleRole::TextInput)
            .accessible_value()
            .unwrap(),
        "Desk left"
    );
}

#[test]
fn an_orientation_asks_first_then_restarts_the_device() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    rig.open_device_settings("Mock Device mock0001");
    let question = "Restart Mock Device mock0001 to switch to Portrait?";

    rig.one("Portrait", AccessibleRole::RadioButton)
        .invoke_accessible_default_action();
    rig.until("the question", |r| r.has(question));
    rig.press("Cancel");
    rig.until("the question to go", |r| !r.has(question));
    assert_eq!(rig.app.ui.get_device_page().orientation, "landscape");
    assert_eq!(
        rig.one("Landscape", AccessibleRole::RadioButton)
            .accessible_checked(),
        Some(true)
    );

    rig.one("Portrait", AccessibleRole::RadioButton)
        .invoke_accessible_default_action();
    rig.until("the question", |r| r.has(question));
    rig.press("Restart");
    rig.until("the restart and the new orientation", |r| {
        let page = r.app.ui.get_device_page();
        page.orientation == "portrait" && page.editable
    });
    assert_eq!(
        rig.deck("mock0001").settings().orientation,
        td_core::Orientation::Portrait
    );
}

#[test]
fn pairings_retry_cancel_and_reconcile() {
    let rig = Rig::start(960.0);
    rig.ports.board("mock0001", BoardKind::DeviceRadio);
    let deck = rig.deck("mock0001");
    deck.add_net_pairing([0x21; 16], "Laptop");
    deck.add_net_pairing([0x42; 16], "Studio PC");
    rig.plug("mock0001", BoardKind::DeviceRadio);
    rig.open_device_settings("Mock Device mock0001");
    rig.until("the paired apps", |r| {
        r.app.ui.get_device_page().pairings_count == "2 of 4"
    });
    let list = rig.one("Paired apps", AccessibleRole::List);
    assert_eq!(list.accessible_item_count(), Some(2));

    // Forget all asks first, and Cancel keeps everything.
    rig.press("Forget all");
    rig.until("the question", |r| r.has("Forget every paired app?"));
    rig.press("Cancel");
    rig.until("the question to go", |r| !r.has("Forget every paired app?"));
    assert_eq!(deck.net_pairings().len(), 2);

    // The answer to a forget is lost: one quiet read settles it.
    deck.lose_net_forget_replies(1);
    rig.press("Forget Laptop");
    rig.until("the reconciled list", |r| {
        r.app.ui.get_device_page().pairings_count == "1 of 4"
    });
    assert_eq!(rig.app.ui.get_device_page().pairings_error, "");
    assert_eq!(deck.net_pairings().len(), 1);

    // The Device can't erase: it says so, and Forget again retries.
    deck.fail_net_storage(true);
    rig.press("Forget Studio PC");
    rig.until("the error", |r| {
        r.app.ui.get_device_page().pairings_error
            == "Mock Device mock0001 couldn't erase that. Try again."
    });
    deck.fail_net_storage(false);
    rig.press("Forget Studio PC");
    rig.until("an empty list", |r| {
        r.app.ui.get_device_page().pairings_count == "0 of 4"
    });
    assert!(rig.text_containing("No apps are paired."));
    assert_eq!(rig.app.ui.get_device_page().pairings_error, "");
}

#[test]
fn a_device_without_pairings_has_no_wireless_group() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    rig.open_device_settings("Mock Device mock0001");
    assert!(!rig.app.ui.get_device_page().pairings_shown);
    assert!(!rig.has("Paired apps"));
    assert!(rig.has("Photo frame"));
}

#[test]
fn the_photo_frame_switch_and_folder_reach_the_device() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().to_str().unwrap().to_owned();
    rig.screens
        .set_picker(std::sync::Arc::new(move || Ok(Some(path.clone()))));
    rig.open_device_settings("Mock Device mock0001");
    rig.one("Photo frame", AccessibleRole::Switch)
        .invoke_accessible_default_action();
    rig.until("the photo frame on", |r| {
        r.app.ui.get_device_page().photo_on
    });
    assert!(rig.deck("mock0001").idle_settings().enabled);
    rig.press("Choose folder");
    let expected = folder.path().to_str().unwrap().to_owned();
    rig.until("the folder", move |r| {
        r.app.ui.get_device_page().folder == expected.as_str()
    });
}

#[test]
fn pc_settings_keep_the_update_switch_and_appearance() {
    let rig = Rig::start(960.0);
    rig.app.ui.set_page(Page::Settings);
    rig.until("the switches", |r| r.has("Check for updates"));
    let updates = rig.one("Check for updates", AccessibleRole::Switch);
    assert_eq!(updates.accessible_checked(), Some(true));
    updates.invoke_accessible_default_action();
    rig.until("the switch off", |r| {
        r.one("Check for updates", AccessibleRole::Switch)
            .accessible_checked()
            == Some(false)
    });
    let dirs = td_core::app_dirs::Dirs {
        config: rig.dir.path().join("config"),
    };
    assert!(
        !td_core::settings::AppSettings::load(&dirs)
            .unwrap()
            .automatic_update_checks
    );

    let start = rig.one("Start at sign-in", AccessibleRole::Switch);
    rig.until("Start at sign-in read", |r| {
        r.one("Start at sign-in", AccessibleRole::Switch)
            .accessible_enabled()
            == Some(true)
    });
    drop(start);
    rig.one("Start at sign-in", AccessibleRole::Switch)
        .invoke_accessible_default_action();
    let handle = rig.handle();
    rig.until("Start at sign-in on", |_| {
        matches!(handle.start_at_sign_in(), Ok(true))
    });

    rig.one("Light", AccessibleRole::RadioButton)
        .invoke_accessible_default_action();
    rig.until("light", |r| !r.app.ui.global::<Theme>().get_dark());
    assert_eq!(
        Prefs::load(&rig.dir.path().join("data")).value.appearance,
        crate::prefs::Appearance::Light
    );
    rig.one("Dark", AccessibleRole::RadioButton)
        .invoke_accessible_default_action();
    rig.until("dark", |r| r.app.ui.global::<Theme>().get_dark());
}

#[test]
fn a_damaged_update_setting_is_left_alone_and_explained() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(dir.path().join("config/app-settings.json"), b"{broken").unwrap();
    let rig = Rig::start(960.0);
    // The rig made its own folder; point a second window at the damaged one.
    crate::ui_tests::init();
    let app = App::new(Options {
        data_dir: dir.path().join("data"),
        config: td_core::app_dirs::Dirs {
            config: dir.path().join("config"),
        },
        close_to_tray: false,
    })
    .unwrap();
    app.ui.window().set_size(LogicalSize::new(960.0, 2000.0));
    app.ui.show().unwrap();
    let _screens = app.connect(&rig.handle(), rig.runtime.handle());
    app.ui.set_page(Page::Settings);
    settle(&app);
    let pc = app.ui.get_pc();
    assert!(!pc.update_enabled);
    assert!(pc.update_problem.contains("doesn't check for updates"));
    assert_eq!(
        std::fs::read(dir.path().join("config/app-settings.json")).unwrap(),
        b"{broken"
    );
}

/// Every control, measured with real fonts, ends inside the window.
fn assert_fits(rig: &Rig, width: f32, page: &str) {
    for role in [
        AccessibleRole::Button,
        AccessibleRole::Tab,
        AccessibleRole::Switch,
        AccessibleRole::RadioButton,
        AccessibleRole::TextInput,
        AccessibleRole::Combobox,
    ] {
        for element in rig
            .app
            .ui
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
        }
    }
}

#[test]
fn at_the_largest_text_size_setup_and_settings_fit_the_narrowest_window() {
    let rig = Rig::start(500.0);
    // Tall enough for the whole page: the search skips what is outside the window.
    rig.app
        .ui
        .window()
        .set_size(LogicalSize::new(500.0, 12000.0));
    look::apply(
        &rig.app.ui,
        &look::Look {
            text_scale: 2.25,
            ..Default::default()
        },
    );
    rig.ports
        .board("mock0001", BoardKind::DeviceRadio)
        .add_net_pairing([0x21; 16], "Laptop");
    rig.plug("mock0001", BoardKind::DeviceRadio);
    rig.ports.plug("mock0009", BoardKind::Silent);
    rig.app.ui.set_page(Page::Setup);
    rig.until("the New board card", |r| r.has("Install Firmware"));
    assert_fits(&rig, 500.0, "Setup");

    rig.open_device_settings("Mock Device mock0001");
    rig.until("the paired apps", |r| r.has("Forget Laptop"));
    // With the photo frame on, its Start after and Folder rows show too.
    rig.one("Photo frame", AccessibleRole::Switch)
        .invoke_accessible_default_action();
    rig.until("the photo frame rows", |r| r.has("Choose folder"));
    assert_fits(&rig, 500.0, "Device settings");
    rig.app.ui.set_settings_device(SharedString::new());
    settle(&rig.app);
    assert_fits(&rig, 500.0, "Settings");
}

#[test]
fn at_the_largest_text_size_every_setup_step_and_button_state_fits() {
    let rig = Rig::start(500.0);
    // Tall enough for the whole page: the search skips what is outside the window.
    rig.app
        .ui
        .window()
        .set_size(LogicalSize::new(500.0, 12000.0));
    look::apply(
        &rig.app.ui,
        &look::Look {
            text_scale: 2.25,
            ..Default::default()
        },
    );
    rig.app.ui.set_page(Page::Setup);
    let wizard = |step: &str, firmware: &str| Wizard {
        id: "mock0009".into(),
        device_name: "Desk".into(),
        device_status: "Connected over USB".into(),
        step: step.into(),
        step_n: 2,
        step_total: 4,
        first: false,
        name: "Desk".into(),
        name_hint: "Stored on the Device, so it keeps its name on any PC.".into(),
        orientation: "landscape".into(),
        firmware_intro: "Desk runs Firmware 1.2.0. Firmware 1.3.0 adds newer Tiles.".into(),
        firmware_step: firmware.into(),
        firmware_status: "Updating… 42%".into(),
        firmware_pct: 42,
        firmware_log: "Sending 42%".into(),
        keep_plugged: "Keep Desk plugged in. It restarts once.".into(),
        live: true,
        done_title: "Desk is ready".into(),
        done_text: "Tap a Tile on Desk to try it.".into(),
        ..Default::default()
    };
    let mut states = Vec::new();
    for step in ["name", "orientation", "done"] {
        states.push((step.to_string(), wizard(step, "idle")));
    }
    for firmware in ["idle", "waiting", "running", "done", "failed", "cancelled"] {
        let mut state = wizard("firmware", firmware);
        state.updating = firmware == "waiting" || firmware == "running";
        states.push((format!("firmware {firmware}"), state));
    }
    // The first step has no Back; a busy step says what it waits for; errors and the offline
    // note take room too.
    let mut first = wizard("firmware", "idle");
    first.first = true;
    states.push(("firmware first".into(), first));
    for busy in ["Saving…", "Restarting…"] {
        let mut state = wizard("orientation", "idle");
        state.busy = busy.into();
        states.push((format!("busy {busy}"), state));
    }
    let mut troubled = wizard("name", "idle");
    troubled.error = "Couldn't save to Desk. Check that it's plugged in, then retry.".into();
    troubled.error_detail = "The Device didn't reply in time.".into();
    troubled.offline = "Plug in Desk to change its settings.".into();
    troubled.live = false;
    states.push(("error and offline".into(), troubled));

    rig.app.ui.set_wizard_open(true);
    for (name, state) in states {
        rig.app.ui.set_wizard(state);
        settle(&rig.app);
        assert_fits(&rig, 500.0, &format!("setup step {name}"));
    }

    rig.app.ui.set_wizard_open(false);
    for step in [
        "detected",
        "waiting",
        "installing",
        "done",
        "failed",
        "declined",
    ] {
        rig.app
            .ui
            .set_boards(slint::ModelRc::new(slint::VecModel::from(vec![
                BoardCard {
                    id: "mock0009".into(),
                    port: "COM12".into(),
                    step: step.into(),
                    status:
                        "Couldn't install Firmware. Unplug the board, plug it back in, then retry."
                            .into(),
                    pct: 42,
                    log: "Failed: no answer".into(),
                },
            ])));
        settle(&rig.app);
        assert_fits(&rig, 500.0, &format!("New board {step}"));
    }
}

#[test]
fn at_the_largest_text_size_every_device_settings_state_fits() {
    let rig = Rig::start(500.0);
    // Tall enough for the whole page: the search skips what is outside the window.
    rig.app
        .ui
        .window()
        .set_size(LogicalSize::new(500.0, 12000.0));
    look::apply(
        &rig.app.ui,
        &look::Look {
            text_scale: 2.25,
            ..Default::default()
        },
    );
    rig.app.ui.set_page(Page::Settings);
    rig.app.ui.set_settings_device("mock0001".into());
    rig.app
        .ui
        .set_paired_apps(slint::ModelRc::new(slint::VecModel::from(vec![
            PairedApp {
                app: "21".repeat(16).into(),
                name: "Laptop".into(),
                this_app: true,
                forgetting: true,
            },
        ])));
    rig.app
        .ui
        .set_after_choices(slint::ModelRc::new(slint::VecModel::from(vec![
            SharedString::from("30 s"),
            SharedString::from("1 min"),
        ])));
    let page = |firmware: &str| DevicePage {
        id: "mock0001".into(),
        name: "Desk".into(),
        status: "Connected over USB".into(),
        offline: "Plug in Desk to change its settings.".into(),
        editable: true,
        name_value: "Desk".into(),
        name_hint: "That name is too long for the Device.".into(),
        name_invalid: true,
        orientation: "landscape".into(),
        confirm: "portrait".into(),
        confirm_title: "Restart Desk to switch to Portrait?".into(),
        photo_on: true,
        photo_enabled: true,
        photo_hint: "Pictures from this PC when Desk sits idle.".into(),
        folder: "C:/Users/Pictures".into(),
        folder_hint: "Nothing is stored on Desk.".into(),
        folder_problem: "That folder isn't on this PC anymore. Choose another.".into(),
        can_pick: true,
        pairings_shown: true,
        pairings_count: "1 of 4".into(),
        pairings_status: "Four apps are paired. Forget one to pair another.".into(),
        pairings_listed: true,
        pairings_editable: true,
        pairings_error: "Desk couldn't erase that. Try again.".into(),
        firmware_title: "Firmware 1.2.0".into(),
        firmware_status: "1.3.0 available".into(),
        firmware_offer: firmware == "idle",
        firmware_usb: true,
        restart_enabled: true,
        firmware_step: firmware.into(),
        firmware_pct: 42,
        firmware_hint: "Keep Desk plugged in. Takes about a minute. Desk restarts once.".into(),
        firmware_log: "Sending 42%".into(),
        notice: "Couldn't save to Desk. Check that it's plugged in, then retry.".into(),
        notice_kind: "error".into(),
        notice_detail: "The Device didn't reply in time.".into(),
        ..Default::default()
    };
    for confirming in [false, true] {
        rig.app.ui.set_confirming_forget_all(confirming);
        for firmware in ["idle", "waiting", "running", "done", "failed", "cancelled"] {
            rig.app.ui.set_device_page(page(firmware));
            settle(&rig.app);
            assert_eq!(
                rig.has("Update"),
                firmware == "idle",
                "Update, Firmware {firmware}"
            );
            assert_fits(
                &rig,
                500.0,
                &format!("Device settings, Firmware {firmware}, forget all {confirming}"),
            );
        }
    }
}
