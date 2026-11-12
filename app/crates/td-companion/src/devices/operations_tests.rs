//! Behaviour through the same Handle interface native screens use, without a web listener.
use super::*;
use crate::{checkpoints::Checkpoint, testing::InProcessCompanion};

async fn radio(td: &InProcessCompanion) -> DeviceId {
    td.ports.plug("mock0001", BoardKind::DeviceRadio);
    let id = DeviceId::parse("mock0001").unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
    })
    .await;
    id
}

#[tokio::test]
async fn settings_coordinate_validation_names_photos_and_restart() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let operations = td.handle.device_operations();
    let initial = operations.settings_get(&id).await.unwrap();
    assert_eq!(initial.orientation, "landscape");
    let result = operations
        .settings_patch(
            &id,
            DeviceSettingsPatch {
                orientation: Some("portrait".into()),
                name: Some("Desk left".into()),
                idle_timeout_s: Some(120),
                photo_frame: Some(PhotoFramePatch {
                    enabled: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.error.is_none());
    assert!(result.pending.is_empty());
    assert!(result.restart_required);
    assert_eq!(
        result.applied,
        [
            SettingsField::IdleTimeout,
            SettingsField::Orientation,
            SettingsField::Name,
            SettingsField::PhotoFrame
        ]
    );
    assert_eq!(result.settings.name.as_deref(), Some("Desk left"));
    assert_eq!(td.device(&id).unwrap().name, result.settings.name);
    assert!(result.settings.photo_frame.unwrap().enabled);
    assert_eq!(td.deck("mock0001").settings().idle_timeout_s, 120);
    let before = td.deck("mock0001").received().len();
    for patch in [
        DeviceSettingsPatch {
            name: Some("bad\u{200b}name".into()),
            ..Default::default()
        },
        DeviceSettingsPatch {
            idle_timeout_s: Some(0),
            ..Default::default()
        },
        DeviceSettingsPatch {
            photo_frame: Some(PhotoFramePatch {
                timeout_s: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            operations.settings_patch(&id, patch).await,
            Err(DeviceError::Common(crate::api::CommonError::Invalid { .. }))
        ));
    }
    assert!(
        td.deck("mock0001").received()[before..]
            .iter()
            .all(|r| r["m"] != "settings.put" && r["m"] != "v2.name.set")
    );
    td.shutdown().await;
}

#[tokio::test]
async fn a_late_failure_keeps_acknowledged_orientation_and_restart() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    // Fail after the Device acknowledges the put, before the requested rename.
    td.handle
        .checkpoints
        .release(Checkpoint::AfterOrientationPut, true);
    let result = td
        .handle
        .device_operations()
        .settings_patch(
            &id,
            DeviceSettingsPatch {
                orientation: Some("portrait".into()),
                name: Some("Later name".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(result.error, Some(DeviceError::Timeout)));
    assert_eq!(result.applied, [SettingsField::Orientation]);
    assert_eq!(result.pending, [SettingsField::Name]);
    assert!(result.restart_required);
    assert_eq!(result.settings.orientation, "portrait");
    assert_eq!(
        td.deck("mock0001").settings().orientation,
        td_core::Orientation::Portrait
    );
    assert_ne!(result.settings.name.as_deref(), Some("Later name"));
    assert_eq!(
        td.handle
            .device_operations()
            .settings_get(&id)
            .await
            .unwrap()
            .orientation,
        "portrait"
    );
    td.shutdown().await;
}

#[tokio::test]
async fn pairings_disconnect_unanswered_forget_storage_failure_reconnect_and_refresh() {
    let mut td = InProcessCompanion::start().await;
    let id = radio(&td).await;
    let operations = td.handle.device_operations();
    let mut changes = operations.observe_pairings();
    let app = [0x20; 16];
    let app_hex = td_link::pairings::app_id_hex(&app);
    td.deck("mock0001").add_net_pairing(app, "Laptop");
    assert_eq!(
        operations.pairings_get(&id).await.unwrap().pairings.len(),
        1
    );
    td.handle.devices().disconnect(&id).unwrap();
    assert!(operations.pairing_snapshot(&id).stale);
    assert!(operations.pairings_forget(&id, &app_hex).await.is_err());
    td.handle.devices().connect(&id).unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
    })
    .await;
    td.deck("mock0001").lose_net_forget_replies(1);
    let forgetting = operations.pairings_forget(&id, &app_hex);
    let observe =
        async {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    changes.changed().await.unwrap();
                    if changes.borrow().get(&id).is_some_and(|s| {
                        s.phase == PairingPhase::Removing && !s.removing.is_empty()
                    }) {
                        break;
                    }
                }
            })
            .await
            .unwrap();
        };
    let (result, ()) = tokio::join!(forgetting, observe);
    assert!(matches!(
        result,
        Err(DevicePairingsError::Device(
            DeviceError::Timeout | DeviceError::UnknownOutcome { .. }
        ))
    ));
    assert_eq!(
        operations.pairing_snapshot(&id).removing.as_slice(),
        std::slice::from_ref(&app_hex)
    );
    // Simulate an unanswered erase that did not persist on the Device, then a storage refusal.
    td.deck("mock0001").add_net_pairing(app, "Laptop");
    td.deck("mock0001").fail_net_storage(true);
    td.unplug("mock0001").await;
    radio(&td).await;
    td.until(|_| {
        let state = operations.pairing_snapshot(&id);
        state.phase == PairingPhase::Ready
            && state.listed.as_ref().is_some_and(|l| l.pairings.len() == 1)
    })
    .await;
    assert_eq!(operations.pairing_snapshot(&id).removing, [app_hex]);
    td.deck("mock0001").fail_net_storage(false);
    assert!(
        operations
            .pairings_get(&id)
            .await
            .unwrap()
            .pairings
            .is_empty()
    );
    let state = operations.pairing_snapshot(&id);
    assert_eq!(state.phase, PairingPhase::Ready);
    assert!(!state.stale);
    assert!(state.removing.is_empty());
    // A later USB reconnect refreshes a cached list even with no pending removals.
    td.unplug("mock0001").await;
    td.deck("mock0001")
        .add_net_pairing([0x30; 16], "Another app");
    radio(&td).await;
    td.until(|_| {
        operations
            .pairing_snapshot(&id)
            .listed
            .as_ref()
            .is_some_and(|l| l.pairings.len() == 1)
    })
    .await;
    td.shutdown().await;
}

#[tokio::test]
async fn removing_this_companion_retries_a_failed_local_key_deletion() {
    let mut td = InProcessCompanion::start().await;
    let fake = td_link::net::fake::FakeNetDevice::start(
        td.ports.board("020000000001", BoardKind::DeviceRadio),
    );
    fake.open_window().unwrap();
    let address = fake.addr().to_string();
    let devices = td.handle.devices().clone();
    let attempt = tokio::task::spawn_blocking(move || devices.wifi_pair_start(&address).unwrap())
        .await
        .unwrap();
    let code = fake.code().unwrap();
    let devices = td.handle.devices().clone();
    tokio::task::spawn_blocking(move || devices.wifi_pair_code(&attempt.attempt, &code).unwrap())
        .await
        .unwrap();
    let id = DeviceId::parse("020000000001").unwrap();
    // The Wi-Fi link comes up first. Plugging USB in while it is still forming lets the fake Device's Wi-Fi session
    // take over the mock deck's one link and close the USB stream.
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| d.carrier == Some(Carrier::Wifi))
    })
    .await;
    td.ports.plug("020000000001", BoardKind::DeviceRadio);
    // Ready for Pairings once USB is the link and the Wi-Fi session is gone.
    td.until(|td| {
        td.device(&id).is_some_and(|d| {
            d.carrier == Some(Carrier::Usb) && matches!(d.link, LinkState::Connected { .. })
        }) && fake.session().is_none()
    })
    .await;
    let operations = td.handle.device_operations();
    let listed = operations.pairings_get(&id).await.unwrap();
    let own = listed
        .pairings
        .iter()
        .find(|p| p.this_app)
        .unwrap()
        .app
        .clone();
    let keys = td.handle.devices().wifi_mock().unwrap().0.clone();
    keys.set_unavailable(true);
    // The Device erases the Pairing, so the forget succeeds and the list works while the key store is away.
    let left = operations.pairings_forget(&id, &own).await.unwrap();
    assert!(left.pairings.is_empty());
    assert!(
        operations
            .pairings_get(&id)
            .await
            .unwrap()
            .pairings
            .is_empty()
    );
    assert!(keys.has(&id));
    let state = operations.pairing_snapshot(&id);
    assert_eq!(state.removing, [own]);
    assert!(state.local_pairing.unwrap().removing);
    assert_eq!(state.phase, PairingPhase::Ready);
    assert!(td.deck("020000000001").net_pairings().is_empty());
    keys.set_unavailable(false);
    operations.pairings_get(&id).await.unwrap();
    assert!(!keys.has(&id));
    let state = operations.pairing_snapshot(&id);
    assert!(state.removing.is_empty());
    assert!(state.local_pairing.is_none());
    td.shutdown().await;
}

#[tokio::test]
async fn photo_folder_failure_keeps_saved_folder_and_acknowledged_device_changes() {
    use crate::companion::{Companion, Config};
    let data = tempfile::tempdir().unwrap();
    // A directory at the persistence destination makes the final atomic rename fail.
    std::fs::create_dir(data.path().join("photo-frame.json")).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let config = Config::mock(data.path().to_owned());
    let ports = config.devices.mock.as_ref().unwrap().0.clone();
    let companion = Companion::start(config).await.unwrap();
    let handle = companion.handle();
    ports.plug("mock0001", BoardKind::Device);
    let id = DeviceId::parse("mock0001").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !handle
            .devices()
            .get(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let patch = DeviceSettingsPatch {
        orientation: Some("portrait".into()),
        photo_frame: Some(PhotoFramePatch {
            enabled: Some(true),
            folder: Some(folder.path().to_str().unwrap().into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let result = handle
        .device_operations()
        .settings_patch(&id, patch.clone())
        .await
        .unwrap();
    assert!(result.error.is_some());
    assert!(result.restart_required);
    assert_eq!(
        result.applied,
        [SettingsField::Orientation, SettingsField::PhotoFrame]
    );
    assert_eq!(result.pending, [SettingsField::PhotoFolder]);
    let frame = result.settings.photo_frame.unwrap();
    assert!(frame.enabled);
    assert!(
        frame.folder.is_none(),
        "a failed save must roll back the PC folder"
    );
    std::fs::remove_dir(data.path().join("photo-frame.json")).unwrap();
    let result = handle
        .device_operations()
        .settings_patch(&id, patch)
        .await
        .unwrap();
    assert!(result.error.is_none());
    assert!(result.pending.is_empty());
    assert_eq!(
        result.settings.photo_frame.unwrap().folder.as_deref(),
        folder.path().to_str()
    );
    companion.shutdown().await;
}
