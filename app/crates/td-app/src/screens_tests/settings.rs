use super::*;

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
