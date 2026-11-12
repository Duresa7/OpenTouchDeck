//! Product behaviour through the same typed seam the native UI will use.
use serde_json::json;
use std::time::{Duration, Instant};
use td_companion::{
    companion::{Change, RecvError},
    devices::{BoardKind, BoardProbe, LinkState},
    testing::InProcessCompanion,
};

#[tokio::test]
async fn two_devices_are_live_at_once_and_told_apart() {
    let td = InProcessCompanion::start().await;
    let started = Instant::now();
    let (a, b) = tokio::join!(td.plug_device("mock0001"), td.plug_device("mock0002"));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let devices = td.handle.snapshot().devices;
    assert_eq!(devices.len(), 2);
    assert_ne!(a, b);
    assert_ne!(devices[0].name, devices[1].name);
    for d in &devices {
        assert!(
            matches!(d.link, LinkState::Connected { protocol: 2, .. }),
            "{d:?}"
        );
        assert!(d.stable_id);
        assert!(d.capabilities.is_some());
    }
    // Unplugging one leaves the other connected.
    td.unplug("mock0001").await;
    let b = td.device(&b).unwrap();
    assert!(matches!(b.link, LinkState::Connected { .. }));
}

#[tokio::test]
async fn a_connected_device_is_probed_once_and_keeps_its_link_across_scans() {
    let td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let hellos = || {
        let received = td.deck("mock0001").received();
        received.iter().filter(|m| m["m"] == "hello").count()
    };
    let probed = hellos();
    let link = td.device(&id).unwrap().link;
    // Longer than two 1 s polls: a scan never reprobes a connected port.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    td.handle.devices().rescan();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(hellos(), probed);
    assert_eq!(td.device(&id).unwrap().link, link);
}

#[tokio::test]
async fn a_silent_port_becomes_a_new_board_after_the_boot_grace() {
    let td = InProcessCompanion::start().await;
    let board = td.plug_board("mock00aa", BoardKind::Silent).await;
    assert_eq!(board.probe, BoardProbe::Silent);
    assert!(td.handle.snapshot().devices.is_empty());
    let boards = td.handle.snapshot().new_boards;
    assert_eq!(boards.len(), 1);
    assert_eq!(boards[0].id, "mock00aa");
}

#[tokio::test]
async fn closed_session_releases_every_held_key() {
    let td = InProcessCompanion::start().await;
    td.plug_device("mock0001").await;
    let deck = td.deck("mock0001");
    deck.emit_key(&json!({"type":"kbd","mods":1,"keys":[4,5]}));
    td.until(|td| {
        td.input_log()
            .iter()
            .filter(|entry| entry.starts_with("down "))
            .count()
            == 3
    })
    .await;
    deck.hang_up();
    td.until(|td| {
        td.input_log()
            .iter()
            .filter(|entry| entry.starts_with("up "))
            .count()
            == 3
    })
    .await;
    let log = td.input_log();
    let mut held = std::collections::BTreeSet::new();
    for entry in log {
        if let Some(key) = entry.strip_prefix("down ") {
            held.insert(key.to_owned());
        } else if let Some(key) = entry.strip_prefix("up ") {
            assert!(held.remove(key), "released a key that was not held");
        }
    }
    assert!(held.is_empty());
}

#[tokio::test]
async fn changes_arrive_in_order_and_operations_need_no_session() {
    let mut td = InProcessCompanion::start().await;
    let mut changes = td.handle.observe();
    assert!(td.handle.snapshot().devices.is_empty());
    let id = td.plug_device("mock0001").await;
    td.handle.devices().disconnect(&id).unwrap();
    let mut added = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match changes.recv().await.unwrap() {
                Change::DeviceAdded(device) if device.0.id == id => {
                    assert!(matches!(device.0.link, LinkState::Connected { .. }));
                    added = true;
                }
                Change::DeviceUpdated(device)
                    if device.0.id == id && device.0.link == LinkState::Disconnected =>
                {
                    assert!(added, "a disconnected update must follow Device appearance");
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(td.device(&id).unwrap().link, LinkState::Disconnected);
    td.handle.devices().connect(&id).unwrap();
    td.until(|td| matches!(td.device(&id).unwrap().link, LinkState::Connected { .. }))
        .await;
    td.shutdown().await;
    assert!(td.handle.shutdown_requested());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(changes.recv().await.unwrap(), Change::Stopping) {}
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn closing_windows_and_observations_keeps_background_work_running() {
    let mut td = InProcessCompanion::start().await;
    let window = td.handle.clone();
    let observation = window.observe();
    drop(observation);
    drop(window);
    let id = td.plug_device("mock0001").await;
    assert!(matches!(
        td.device(&id).unwrap().link,
        LinkState::Connected { .. }
    ));
    assert!(!td.handle.shutdown_requested());
    td.deck("mock0001")
        .emit_key(&json!({"type":"kbd","mods":0,"keys":[4]}));
    td.until(|td| td.input_log().iter().any(|entry| entry == "down A"))
        .await;
    td.drop_owner();
    assert!(td.handle.shutdown_requested());
    td.until(|td| td.input_log().iter().any(|entry| entry == "up A"))
        .await;
}

#[tokio::test]
async fn a_slow_observer_can_resubscribe_and_read_current_state() {
    let mut td = InProcessCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    td.unplug("mock0001").await;
    let mut changes = td.handle.observe();
    for _ in 0..1100 {
        td.handle.devices().connect(&id);
        td.handle.devices().disconnect(&id);
    }
    assert!(matches!(changes.recv().await, Err(RecvError::Lagged(_))));
    let _fresh = td.handle.observe();
    assert_eq!(
        td.handle.snapshot().devices[0].link,
        LinkState::Disconnected
    );
    td.shutdown().await;
}
