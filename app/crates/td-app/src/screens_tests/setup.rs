use super::*;

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
