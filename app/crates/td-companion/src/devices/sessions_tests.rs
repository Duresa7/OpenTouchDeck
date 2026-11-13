//! The screens' sessions through the in-process Companion and mock Devices: the rules the 1.x
//! TypeScript kept (rollback, pending fields, orientation confirmation, restart, Pairing retry
//! and reconciliation, setup and New board installs), now behind Rust interfaces.
use super::*;
use crate::{
    checkpoints::Checkpoint,
    features::firmware::{BoardStep, ConsentAnswer, FirmwareTask, JobView, Target, board_step},
    setup::{Busy, SetupSession, Step, candidate},
    testing::InProcessCompanion,
};
use std::time::Duration;

async fn until<T: Clone>(rx: &mut tokio::sync::watch::Receiver<T>, done: impl Fn(&T) -> bool) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            {
                let value = rx.borrow_and_update().clone();
                if done(&value) {
                    return value;
                }
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("the session settled in time")
}

async fn radio(td: &InProcessCompanion, serial: &str) -> DeviceId {
    td.ports.plug(serial, BoardKind::DeviceRadio);
    let id = DeviceId::parse(serial).unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
    })
    .await;
    id
}

#[tokio::test]
async fn a_rename_is_checked_here_then_saved_or_rolled_back() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let session = td.handle.device_operations().settings_session(&id);
    session.load().await;
    let view = session.view();
    assert_eq!(view.load, Load::Ready);
    assert!(view.editable());
    assert!(!view.needs.name);
    let before = td.deck("mock0001").received().len();

    assert!(!session.rename("   ").await);
    assert_eq!(
        session.view().name_problem.as_deref(),
        Some("Give the Device a name.")
    );
    assert!(!session.rename("tab\there").await);
    assert_eq!(td.deck("mock0001").received().len(), before, "nothing sent");

    assert!(session.rename("Desk left").await);
    let view = session.view();
    assert_eq!(view.name_problem, None);
    assert_eq!(view.saved.unwrap().name.as_deref(), Some("Desk left"));
    assert_eq!(view.notice.unwrap().event, "saved");
    assert_eq!(td.device(&id).unwrap().name.as_deref(), Some("Desk left"));

    // The Device stops answering: the name shows at once, then rolls back to the saved one.
    td.deck("mock0001").delay_replies(Duration::from_secs(3));
    let mut views = session.subscribe();
    let renaming = tokio::spawn({
        let session = session.clone();
        async move { session.rename("Too slow").await }
    });
    until(&mut views, |v| {
        v.shown.as_ref().and_then(|s| s.name.as_deref()) == Some("Too slow")
    })
    .await;
    assert!(!renaming.await.unwrap());
    let view = session.view();
    assert_eq!(view.shown.unwrap().name.as_deref(), Some("Desk left"));
    let notice = view.notice.unwrap();
    assert_eq!(notice.kind, NoticeKind::Error);
    assert_eq!(notice.event, "save-failed");
    assert_eq!(
        notice.text,
        "Couldn't save to Desk left. Check that it's plugged in, then retry."
    );
    assert!(notice.detail.is_some());
    td.shutdown().await;
}

#[tokio::test]
async fn a_pending_field_keeps_its_change_while_another_field_is_answered() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let session = td.handle.device_operations().settings_session(&id);
    session.load().await;
    let old = session.view().saved.unwrap().name.unwrap();
    let checkpoints = td.handle.checkpoints.clone();
    checkpoints.hold(Checkpoint::BeforeOrientationPut);
    session.choose_orientation(td_core::Orientation::Portrait);
    let orienting = tokio::spawn({
        let session = session.clone();
        async move { session.confirm_orientation().await }
    });
    td.until(|_| checkpoints.waiting(Checkpoint::BeforeOrientationPut))
        .await;
    let mut seen = session.subscribe();
    let renaming = tokio::spawn({
        let session = session.clone();
        async move { session.rename("Pending name").await }
    });
    until(&mut seen, |v| {
        v.shown.as_ref().and_then(|s| s.name.as_deref()) == Some("Pending name")
    })
    .await;
    // Every view from here on, including the orientation's answer, keeps the pending name.
    let mut watch = session.subscribe();
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let recorder = tokio::spawn(async move {
        let mut names = Vec::new();
        loop {
            tokio::select! {
                _ = &mut stopped => return names,
                changed = watch.changed() => {
                    changed.unwrap();
                    names.push(watch.borrow_and_update().shown.as_ref().and_then(|s| s.name.clone()));
                }
            }
        }
    });
    checkpoints.release(Checkpoint::BeforeOrientationPut, false);
    assert!(renaming.await.unwrap());
    orienting.await.unwrap();
    stop.send(()).unwrap();
    let names = recorder.await.unwrap();
    assert!(!names.is_empty());
    assert!(
        names
            .iter()
            .all(|name| name.as_deref() == Some("Pending name")),
        "{names:?} never shows {old:?} again"
    );
    let view = session.view();
    assert_eq!(view.restart, Restart::Idle);
    let shown = view.shown.unwrap();
    assert_eq!(shown.name.as_deref(), Some("Pending name"));
    assert_eq!(shown.orientation, "portrait");
    td.shutdown().await;
}

#[tokio::test]
async fn an_orientation_waits_for_confirmation_then_restarts_the_device() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let session = td.handle.device_operations().settings_session(&id);
    session.load().await;
    assert_eq!(session.view().shown.unwrap().orientation, "landscape");

    session.choose_orientation(td_core::Orientation::Landscape);
    assert_eq!(session.view().confirm, None, "already landscape");
    session.choose_orientation(td_core::Orientation::Portrait);
    assert_eq!(session.view().confirm, Some(td_core::Orientation::Portrait));
    session.cancel_orientation();
    assert_eq!(session.view().confirm, None);
    session.confirm_orientation().await;
    assert_eq!(
        td.deck("mock0001").settings().orientation,
        td_core::Orientation::Landscape,
        "nothing to confirm, nothing sent"
    );

    session.choose_orientation(td_core::Orientation::Portrait);
    let reboots = td.deck("mock0001").reboots();
    let mut views = session.subscribe();
    let confirming = tokio::spawn({
        let session = session.clone();
        async move { session.confirm_orientation().await }
    });
    let restarting = until(&mut views, |v| v.restart == Restart::Restarting).await;
    assert!(!restarting.editable());
    assert_eq!(restarting.notice.unwrap().text, "Restarting…");
    confirming.await.unwrap();
    let view = session.view();
    assert_eq!(view.restart, Restart::Idle);
    assert_eq!(view.load, Load::Ready, "read again after the restart");
    assert_eq!(view.notice, None);
    assert_eq!(view.shown.unwrap().orientation, "portrait");
    assert_eq!(
        td.deck("mock0001").settings().orientation,
        td_core::Orientation::Portrait
    );
    let _ = reboots;
    td.shutdown().await;
}

#[tokio::test]
async fn a_restart_that_never_comes_back_fails() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let session = SettingsSession::with_restart_timeout(
        td.handle.device_operations().clone(),
        id.clone(),
        Duration::from_millis(500),
    );
    let mut views = session.subscribe();
    let restarting = tokio::spawn({
        let session = session.clone();
        async move { session.restart().await }
    });
    until(&mut views, |v| v.restart == Restart::Restarting).await;
    td.ports.unplug("mock0001");
    restarting.await.unwrap();
    let view = session.view();
    assert_eq!(view.restart, Restart::Failed);
    let notice = view.notice.unwrap();
    assert_eq!(notice.event, "restart-failed");
    assert!(notice.text.ends_with("Unplug it and plug it back in."));
    td.shutdown().await;
}

#[tokio::test]
async fn v1_firmware_cannot_take_a_name() {
    let mut td = InProcessCompanion::start().await;
    td.ports.plug("mock0001", BoardKind::DeviceV1);
    let id = DeviceId::parse("mock0001").unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
    })
    .await;
    let session = td.handle.device_operations().settings_session(&id);
    session.load().await;
    let view = session.view();
    assert!(view.needs.name);
    assert!(view.needs.photo_frame);
    assert!(!session.rename("Desk").await);
    td.shutdown().await;
}

#[tokio::test]
async fn pairings_forget_retries_and_reconciles_an_unanswered_forget() {
    let mut td = InProcessCompanion::start().await;
    let id = radio(&td, "mock0001").await;
    let deck = td.deck("mock0001");
    let laptop = [0x20; 16];
    deck.add_net_pairing(laptop, "Laptop");
    deck.add_net_pairing([0x30; 16], "Phone");
    let session = td.handle.device_operations().pairings_session(&id);
    assert!(offers_pairings(&td.device(&id).unwrap()));
    session.load().await;
    let view = session.view();
    assert_eq!(view.load, Load::Ready);
    assert_eq!(view.count().as_deref(), Some("2 of 4"));
    assert_eq!(view.status("Desk", true, true), None);
    assert!(view.editable(true));
    assert!(!view.editable(false));

    // The answer to the forget is lost. One quiet read sends it again, settling it.
    deck.lose_net_forget_replies(1);
    let app = td_link::pairings::app_id_hex(&laptop);
    assert!(!session.forget(&app).await);
    let view = session.view();
    assert_eq!(view.error, None, "the read settled it");
    assert_eq!(view.forgetting, None);
    assert_eq!(view.pairings.as_ref().unwrap().pairings.len(), 1);
    assert_eq!(deck.net_pairings().len(), 1);

    // The Device can't erase: the error says so, and pressing Forget again retries.
    deck.fail_net_storage(true);
    let phone = td_link::pairings::app_id_hex(&[0x30; 16]);
    assert!(!session.forget(&phone).await);
    let view = session.view();
    assert_eq!(
        view.error.as_deref(),
        Some("Mock Device mock0001 couldn't erase that. Try again.")
    );
    deck.fail_net_storage(false);
    assert!(session.forget(&phone).await);
    let view = session.view();
    assert_eq!(view.error, None);
    assert_eq!(
        view.status("Desk", true, true).as_deref(),
        Some("No apps are paired. Pair one over Wi-Fi to use Desk without the cable.")
    );

    // Forget all, with four paired: the list is full first.
    for n in 0..4u8 {
        deck.add_net_pairing([0x40 + n; 16], "App");
    }
    session.load().await;
    assert_eq!(
        session.view().status("Desk", true, true).as_deref(),
        Some("Four apps are paired. Forget one to pair another.")
    );
    assert!(session.forget_all().await);
    assert!(deck.net_pairings().is_empty());
    assert_eq!(session.view().count().as_deref(), Some("0 of 4"));
    td.shutdown().await;
}

#[tokio::test]
async fn pairings_wait_for_usb_and_reload_when_it_returns() {
    let mut td = InProcessCompanion::start().await;
    let id = radio(&td, "mock0001").await;
    let session = td.handle.device_operations().pairings_session(&id);
    td.handle.devices().disconnect(&id).unwrap();
    let device = td.device(&id).unwrap();
    assert!(!session.observe(&device));
    session.load().await;
    assert_eq!(session.view().load, Load::Idle, "nothing read without USB");
    assert!(!session.forget_all().await);
    assert_eq!(
        session.view().status("Desk", false, false).as_deref(),
        Some("Plug in Desk to see its paired apps.")
    );
    td.handle.devices().connect(&id).unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
    })
    .await;
    assert!(session.observe(&td.device(&id).unwrap()), "back over USB");
    assert!(!session.observe(&td.device(&id).unwrap()), "once");

    // A Device without Pairings is never asked.
    let plain = td.plug_device("mock0002").await;
    let other = td.handle.device_operations().pairings_session(&plain);
    assert!(!offers_pairings(&td.device(&plain).unwrap()));
    other.load().await;
    assert_eq!(other.view().load, Load::Idle);
    td.shutdown().await;
}

#[tokio::test]
async fn a_new_board_installs_after_a_declined_try_then_setup_names_and_turns_it() {
    let mut td = InProcessCompanion::start().await;
    let board = td.plug_board("mock0009", BoardKind::Silent).await;
    let install = FirmwareTask::new(td.handle.clone(), Target::NewBoard(board.id.clone()));

    td.handle.firmware().script(vec![ConsentAnswer::Decline]);
    install.start().await;
    let view = install.view();
    assert_eq!(board_step(&view.view), BoardStep::Declined);
    assert_eq!(
        view.view.text(),
        "Declined on this PC. Nothing changed on the board."
    );
    assert!(view.log.iter().any(|line| line.starts_with("Failed:")));

    // Retry.
    td.handle.firmware().script(vec![ConsentAnswer::Accept]);
    install.start().await;
    let view = install.view();
    assert!(matches!(view.view, JobView::Done { .. }), "{view:?}");
    assert_eq!(view.view.text(), "Firmware installed");
    assert!(
        view.log
            .iter()
            .any(|line| line == "Waiting for confirmation"),
        "{:?}",
        view.log
    );

    let id = board.id.clone();
    td.until(|td| {
        td.device(&id).is_some_and(|d| {
            d.setup == SetupState::NeedsSetup && matches!(d.link, LinkState::Connected { .. })
        })
    })
    .await;
    let devices = td.handle.snapshot().devices;
    let skips = td.handle.setup_skips();
    let device = candidate(&devices, &skips.skipped()).unwrap().clone();
    assert_eq!(device.id, id);

    let setup = SetupSession::new(td.handle.device_operations().clone(), device, false);
    assert_eq!(
        setup.view().flow.steps,
        [Step::Name, Step::Orientation, Step::Done]
    );
    setup.set_name("  ");
    setup.submit_name().await;
    assert_eq!(setup.view().name_error.as_deref(), Some("Enter a name."));
    assert_eq!(setup.view().flow.at, Step::Name);
    setup.set_name(" Desk ");
    assert_eq!(setup.view().name_error, None);
    setup.submit_name().await;
    let view = setup.view();
    assert_eq!(view.flow.at, Step::Orientation);
    assert_eq!(view.name, "Desk");
    let named = td.device(&id).unwrap();
    assert_eq!(named.name.as_deref(), Some("Desk"));
    assert_eq!(named.setup, SetupState::Ready, "named, so set up");
    setup.observe(&named);

    setup.choose_orientation(td_core::Orientation::Portrait);
    let mut views = setup.subscribe();
    let turning = tokio::spawn({
        let setup = setup.clone();
        async move { setup.submit_orientation().await }
    });
    until(&mut views, |v| v.busy == Some(Busy::Restarting)).await;
    turning.await.unwrap();
    let view = setup.view();
    assert_eq!(view.busy, None);
    assert_eq!(view.error, None);
    assert_eq!(view.flow.at, Step::Done);
    assert_eq!(
        td.deck("mock0009").settings().orientation,
        td_core::Orientation::Portrait
    );

    setup.back();
    assert_eq!(setup.view().flow.at, Step::Orientation);
    td.shutdown().await;
}

#[tokio::test]
async fn a_closed_setup_is_skipped_until_the_board_is_installed_again() {
    let mut td = InProcessCompanion::start().await;
    let board = td.plug_board("mock0009", BoardKind::Silent).await;
    td.handle.firmware().script(vec![ConsentAnswer::Accept]);
    let install = FirmwareTask::new(td.handle.clone(), Target::NewBoard(board.id.clone()));
    install.start().await;
    let id = board.id.clone();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| d.setup == SetupState::NeedsSetup)
    })
    .await;
    let skips = td.handle.setup_skips();
    skips.closed(td.device(&id).as_ref());
    let devices = td.handle.snapshot().devices;
    assert!(candidate(&devices, &skips.skipped()).is_none());
    skips.unskip(&id);
    assert!(candidate(&devices, &skips.skipped()).is_some());
    td.shutdown().await;
}

#[tokio::test]
async fn setup_moves_on_when_the_firmware_cannot_keep_a_name() {
    let mut td = InProcessCompanion::start().await;
    td.ports.plug("mock0001", BoardKind::DeviceV1);
    let id = DeviceId::parse("mock0001").unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
    })
    .await;
    let setup = SetupSession::new(
        td.handle.device_operations().clone(),
        td.device(&id).unwrap(),
        true,
    );
    // Firmware too old for a name goes first.
    assert_eq!(setup.view().flow.at, Step::Firmware);
    setup.next();
    setup.set_name("Desk");
    setup.submit_name().await;
    assert!(setup.view().needs_firmware);
    assert_eq!(setup.view().flow.at, Step::Name);
    setup.submit_name().await;
    assert_eq!(setup.view().flow.at, Step::Orientation);
    td.shutdown().await;
}

#[tokio::test]
async fn a_device_on_older_firmware_of_its_build_is_offered_the_bundled_one() {
    use crate::features::firmware::{FirmwareBuild, RadioBuild, offered};
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let mut device = td.device(&id).unwrap();
    let builds = FirmwareBuild {
        version: "1.3.0".into(),
        build: String::new(),
        radio: Some(RadioBuild {
            version: "1.4.0".into(),
            build: String::new(),
        }),
    };
    let firmware = device.firmware.as_mut().unwrap();
    firmware.version = "1.2.9".into();
    assert_eq!(offered(&device, Some(&builds)).as_deref(), Some("1.3.0"));
    device.firmware.as_mut().unwrap().channel = FirmwareChannel::Radio;
    assert_eq!(offered(&device, Some(&builds)).as_deref(), Some("1.4.0"));
    device.firmware.as_mut().unwrap().version = "1.4.0".into();
    assert_eq!(offered(&device, Some(&builds)), None, "already on it");
    device.firmware.as_mut().unwrap().version = "mock".into();
    assert_eq!(offered(&device, Some(&builds)), None, "not a version");
    device.firmware.as_mut().unwrap().update_available = Some("2.0.0".into());
    assert_eq!(offered(&device, None).as_deref(), Some("2.0.0"));
    td.shutdown().await;
}
