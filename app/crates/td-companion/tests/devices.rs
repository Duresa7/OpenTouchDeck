//! The Device manager through the API: detection, hot-plug, New boards, the console allowlist,
//! opt-in topics and the key mirror.
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use td_companion::{
    api::NoQuery,
    devices::{BoardKind, BoardProbe, DeviceId, LinkState},
    features::devices::{
        Connect, DevicePath, DeviceSettingsPatch, Disconnect, Get, NewBoardsList, PortsList,
        Restart, SettingsGet, SettingsPatch,
    },
    testing::TestCompanion,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// The next frame `pick` accepts within `within`, skipping the others.
async fn frame(socket: &mut Socket, within: Duration, pick: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(within, async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Ok(text) = message.to_text()
                && let Ok(frame) = serde_json::from_str::<Value>(text)
                && pick(&frame)
            {
                return frame;
            }
        }
    })
    .await
    .expect("no matching frame in time")
}
fn device_update(id: &str, state: &'static str) -> impl Fn(&Value) -> bool {
    let id = id.to_owned();
    move |f| {
        matches!(f["type"].as_str(), Some("device.updated" | "device.added"))
            && f["device"] == id.as_str()
            && f["data"]["link"]["state"] == state
    }
}
fn path(id: &DeviceId) -> DevicePath {
    DevicePath { device: id.clone() }
}
async fn topic(socket: &mut Socket, op: &str, topic: &str, device: &str) {
    let request = json!({"op": op, "topic": topic, "device": device});
    socket
        .send(Message::text(request.to_string()))
        .await
        .unwrap();
}
/// POSTs a console line and returns the status and error code.
async fn console(td: &TestCompanion, route: &str, line: &str) -> (u16, Value) {
    let response = td
        .raw()
        .post(route)
        .bearer_auth(&td.token)
        .json(&json!({ "line": line }))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    (status, body["error"]["code"].clone())
}

#[tokio::test]
async fn unplug_is_lost_and_replug_reconnects_within_two_seconds() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let mut socket = td.events().await;
    td.unplug("mock0001").await;
    let lost = frame(
        &mut socket,
        Duration::from_secs(2),
        device_update("mock0001", "lost"),
    )
    .await;
    assert_eq!(lost["data"]["link"]["reason"], "unplugged");
    let started = Instant::now();
    td.plug_device("mock0001").await;
    frame(
        &mut socket,
        Duration::from_secs(2),
        device_update("mock0001", "connected"),
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(2));
    let summary = td.call::<Get>(path(&id), NoQuery, ()).await.unwrap();
    assert!(summary.rev > 1);
}

#[tokio::test]
async fn a_lost_link_reconnects_and_reports_why() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let mut socket = td.events().await;
    // The stream ends while the port stays: the link is lost, then the Device reconnects.
    td.deck("mock0001").hang_up();
    let lost = frame(
        &mut socket,
        Duration::from_secs(2),
        device_update("mock0001", "lost"),
    )
    .await;
    assert_eq!(lost["data"]["link"]["reason"], "closed");
    frame(
        &mut socket,
        Duration::from_secs(2),
        device_update("mock0001", "connected"),
    )
    .await;
    assert!(matches!(
        td.device(&id).unwrap().link,
        LinkState::Connected { .. }
    ));
}

#[tokio::test]
async fn disconnect_holds_until_connect_or_replug() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let summary = td.call::<Disconnect>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(summary.link, LinkState::Disconnected);
    // Longer than the 1 s poll: auto-connect stays off.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(td.device(&id).unwrap().link, LinkState::Disconnected);
    let response = td
        .raw()
        .post("/api/v1/devices/mock0001/console")
        .bearer_auth(&td.token)
        .json(&json!({"line": "version"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "device.not_connected");
    assert_eq!(body["error"]["details"]["link"]["state"], "disconnected");
    td.call::<Connect>(path(&id), NoQuery, ()).await.unwrap();
    td.until(|td| matches!(td.device(&id).unwrap().link, LinkState::Connected { .. }))
        .await;
    // Replug also turns auto-connect back on.
    td.call::<Disconnect>(path(&id), NoQuery, ()).await.unwrap();
    td.unplug("mock0001").await;
    assert_eq!(td.device(&id).unwrap().link, LinkState::Offline);
    td.plug_device("mock0001").await;
}

#[tokio::test]
async fn other_firmware_is_a_new_board_until_unplugged() {
    let td = TestCompanion::start().await;
    let mut socket = td.events().await;
    td.plug_board("mock00ff", BoardKind::OtherFirmware).await;
    let added = frame(&mut socket, Duration::from_secs(2), |f| {
        f["type"] == "new-board.added"
    })
    .await;
    assert_eq!(added["data"]["id"], "mock00ff");
    assert_eq!(added["data"]["probe"], "other-firmware");
    assert_eq!(added["data"]["product"], "esp-at");
    td.unplug("mock00ff").await;
    let removed = frame(&mut socket, Duration::from_secs(2), |f| {
        f["type"] == "new-board.removed"
    })
    .await;
    assert_eq!(removed["data"]["id"], "mock00ff");
    assert!(
        td.call::<NewBoardsList>((), NoQuery, ())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_busy_port_is_a_busy_new_board_or_a_busy_device() {
    let td = TestCompanion::start().await;
    let board = td.plug_board("mock00bb", BoardKind::Busy).await;
    assert!(matches!(board.probe, BoardProbe::Busy { .. }));
    let ports = td.call::<PortsList>((), NoQuery, ()).await.unwrap();
    let raw = serde_json::to_value(&ports).unwrap();
    assert_eq!(raw[0]["status"], "busy");
    assert_eq!(raw[0]["usb_serial"], "mock00bb");
    // A known Device whose port another program holds is a busy Device, not a New board.
    let id = td.plug_device("mock0001").await;
    td.unplug("mock0001").await;
    plug_busy(&td, "mock0001").await;
    td.until(|td| matches!(td.device(&id).unwrap().link, LinkState::Busy { .. }))
        .await;
    assert!(
        !td.call::<NewBoardsList>((), NoQuery, ())
            .await
            .unwrap()
            .iter()
            .any(|b| b.id == "mock0001")
    );
}
async fn plug_busy(td: &TestCompanion, id: &str) {
    let response = td
        .raw()
        .post("/api/v1/test/ports")
        .bearer_auth(&td.token)
        .json(&json!({"plug": {"id": id, "kind": "busy"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
}

#[tokio::test]
async fn v1_firmware_connects_without_capabilities() {
    let td = TestCompanion::start().await;
    let id = td.plug_device_v1("mock0003").await;
    let summary = td.call::<Get>(path(&id), NoQuery, ()).await.unwrap();
    assert!(matches!(
        summary.link,
        LinkState::Connected { protocol: 1, .. }
    ));
    assert_eq!(summary.capabilities, None);
    assert_eq!(summary.name, None);
    assert_eq!(summary.firmware.unwrap().protocol, 1);
    // A v2 field on v1 Firmware needs newer Firmware.
    let response = td
        .raw()
        .request(reqwest::Method::PATCH, "/api/v1/devices/mock0003/settings")
        .bearer_auth(&td.token)
        .json(&json!({"name": "Desk"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "device.needs_firmware");
    assert_eq!(body["error"]["details"]["field"], "name");
    let settings = td
        .call::<SettingsGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(settings.name, None);
    assert_eq!(settings.brightness, None);
}

#[tokio::test]
async fn settings_and_restart_reach_the_device() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let settings = td
        .call::<SettingsGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(settings.orientation, "landscape");
    assert_eq!(settings.name.as_deref(), Some("Mock Device mock0001"));
    let patch = DeviceSettingsPatch {
        idle_timeout_s: Some(120),
        orientation: Some("portrait".into()),
        name: Some("Desk left".into()),
        ..Default::default()
    };
    let patched = td
        .call::<SettingsPatch>(path(&id), NoQuery, patch)
        .await
        .unwrap();
    assert!(patched.restart_required);
    assert_eq!(patched.settings.idle_timeout_s, 120);
    assert_eq!(patched.settings.orientation, "portrait");
    assert_eq!(patched.settings.name.as_deref(), Some("Desk left"));
    assert_eq!(td.deck("mock0001").settings().idle_timeout_s, 120);
    assert_eq!(td.device(&id).unwrap().name.as_deref(), Some("Desk left"));
    for (body, code) in [
        (json!({"orientation": "sideways"}), "request.invalid"),
        (json!({"idle_timeout_s": 0}), "request.invalid"),
        (json!({"brightness": 50}), "device.needs_firmware"),
        (json!({"brightness": 100}), "device.needs_firmware"),
        (json!({"brightness": 101}), "request.invalid"),
    ] {
        let response = td
            .raw()
            .request(reqwest::Method::PATCH, "/api/v1/devices/mock0001/settings")
            .bearer_auth(&td.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], code);
    }
    // Restart closes the link and comes back with a fresh hello.
    let mut socket = td.events().await;
    let hellos = || {
        let received = td.deck("mock0001").received();
        received.iter().filter(|m| m["m"] == "hello").count()
    };
    let before = hellos();
    td.call::<Restart>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(td.deck("mock0001").reboots(), 1);
    frame(
        &mut socket,
        Duration::from_secs(2),
        device_update("mock0001", "connecting"),
    )
    .await;
    frame(
        &mut socket,
        Duration::from_secs(5),
        device_update("mock0001", "connected"),
    )
    .await;
    assert!(hellos() > before);
}

#[tokio::test]
async fn a_restart_the_device_starts_itself_brings_a_fresh_hello() {
    let td = TestCompanion::start().await;
    td.plug_device("mock0001").await;
    let mut socket = td.events().await;
    // One ping every 2 s gives the uptime a baseline first.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    // The full console reaches the mock, which restarts; the next ping sees its uptime reset.
    let (status, _) = console(&td, "/api/v1/test/devices/mock0001/console", "reboot").await;
    assert_eq!(status, 200);
    let lost = frame(
        &mut socket,
        Duration::from_secs(5),
        device_update("mock0001", "lost"),
    )
    .await;
    assert_eq!(lost["data"]["link"]["detail"], "the deck restarted");
    frame(
        &mut socket,
        Duration::from_secs(5),
        device_update("mock0001", "connected"),
    )
    .await;
}

#[tokio::test]
async fn the_console_allowlist_is_enforced_before_the_device() {
    let td = TestCompanion::start().await;
    td.plug_device("mock0001").await;
    let console_lines = || {
        td.deck("mock0001")
            .lines()
            .into_iter()
            .filter(|line| line != "layout state")
            .collect::<Vec<_>>()
    };
    let route = "/api/v1/devices/mock0001/console";
    assert_eq!(console(&td, route, "version").await.0, 204);
    assert_eq!(console(&td, route, "link state").await.0, 204);
    td.until(|_| console_lines() == ["version", "link state"])
        .await;
    let long = format!("store get /{}", "a".repeat(300));
    for line in [
        "keys a",
        "touch tap 10 10",
        "ui hold launch",
        "reboot",
        "version extra",
        "nonsense",
        "version\u{1b}",
        "help\tversion",
        long.as_str(),
    ] {
        let (status, code) = console(&td, route, line).await;
        assert_eq!(
            (status, code.as_str()),
            (403, Some("console.forbidden")),
            "{line:?}"
        );
    }
    td.let_live_rounds_run(1).await;
    assert!(
        td.deck("mock0001")
            .lines()
            .iter()
            .any(|l| l == "layout state")
    );
    assert_eq!(console_lines(), ["version", "link state"]);
    assert!(td.input_log().is_empty());
    assert_eq!(
        console(&td, "/api/v1/test/devices/mock0001/console", "keys a")
            .await
            .0,
        200
    );
    td.until(|_| console_lines() == ["version", "link state", "keys a"])
        .await;
}

async fn test_console(td: &TestCompanion, device: &str, body: Value) -> (u16, Value) {
    let response = td
        .raw()
        .post(&format!("/api/v1/test/devices/{device}/console"))
        .bearer_auth(&td.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (
        status,
        response.json::<Value>().await.unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn test_console_returns_what_the_device_answered() {
    let td = TestCompanion::start().await;
    td.plug_device("mock0001").await;
    let (status, body) = test_console(&td, "mock0001", json!({"line": "version"})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["reply"]["cmd"], "version");
    assert_eq!(body["reply"]["ok"], true);
    assert_eq!(body["reply"]["project_name"], "touchdeck");
    let lines = body["lines"].as_array().unwrap();
    assert!(lines.last().unwrap().as_str().unwrap().contains("@td "));
    let (status, body) = test_console(&td, "mock0001", json!({"line": "keys a"})).await;
    assert_eq!(status, 200);
    assert_eq!(body["reply"]["cmd"], "keys");
    assert_eq!(body["reply"]["ok"], false);
}

#[tokio::test]
async fn test_console_waits_for_a_reply_only_until_its_deadline() {
    let td = TestCompanion::start().await;
    td.plug_device("mock0001").await;
    td.deck("mock0001").lose_console_replies(1);
    let started = Instant::now();
    let (status, body) =
        test_console(&td, "mock0001", json!({"line": "version", "wait_ms": 300})).await;
    let elapsed = started.elapsed();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["reply"], Value::Null);
    assert!(
        (Duration::from_millis(300)..Duration::from_millis(1500)).contains(&elapsed),
        "{elapsed:?}"
    );
    assert!(td.deck("mock0001").lines().iter().any(|l| l == "version"));
    let (status, body) = test_console(
        &td,
        "mock0001",
        json!({"line": "version", "wait_ms": 15_001}),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    td.deck("mock0001").lose_console_replies(1);
    let started = Instant::now();
    let (status, body) =
        test_console(&td, "mock0001", json!({"line": "version", "wait_ms": 0})).await;
    assert_eq!((status, &body["reply"]), (200, &Value::Null));
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn test_console_is_not_found_for_a_real_port_device_without_the_opt_in() {
    let td = TestCompanion::start_on_real_ports(false).await;
    td.plug_device("mock0001").await;
    let (status, body) = test_console(&td, "mock0001", json!({"line": "touch tap 10 10"})).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (404, Some("route.not_found"))
    );
    td.let_live_rounds_run(1).await;
    assert!(
        !td.deck("mock0001")
            .lines()
            .iter()
            .any(|l| l.starts_with("touch"))
    );
}

#[tokio::test]
async fn test_console_reaches_a_real_port_device_with_the_opt_in() {
    let td = TestCompanion::start_on_real_ports(true).await;
    td.plug_device("mock0001").await;
    let (status, body) = test_console(&td, "mock0001", json!({"line": "version"})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["reply"]["cmd"], "version");
    assert!(td.deck("mock0001").lines().iter().any(|l| l == "version"));
    let (status, _) = test_console(&td, "nosuch01", json!({"line": "version"})).await;
    assert_eq!(status, 404);
    let response = td
        .raw()
        .get("/api/v1/test/devices/mock0001/record")
        .bearer_auth(&td.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);
}

#[tokio::test]
async fn the_release_router_has_no_test_console() {
    let td = TestCompanion::start_release_router().await;
    td.plug_device("mock0001").await;
    let (status, body) = test_console(&td, "mock0001", json!({"line": "version"})).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (404, Some("route.not_found"))
    );
}

#[tokio::test]
async fn subscribe_and_unsubscribe_gate_device_console() {
    let td = TestCompanion::start().await;
    td.plug_device("mock0001").await;
    let mut socket = td.events().await;
    let route = "/api/v1/devices/mock0001/console";
    topic(&mut socket, "subscribe", "console", "mock0001").await;
    // The subscription lands on the socket's own task; retry until a line arrives.
    let line = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            console(&td, route, "version").await;
            let got = tokio::time::timeout(Duration::from_millis(300), async {
                frame(&mut socket, Duration::from_secs(1), |f| {
                    f["type"] == "device.console"
                })
                .await
            })
            .await;
            if let Ok(frame) = got {
                break frame;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(line["data"]["device"], "mock0001");
    assert!(line["data"]["line"].as_str().unwrap().contains("@td "));
    topic(&mut socket, "unsubscribe", "console", "mock0001").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Drain lines already queued before the unsubscribe.
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(100), socket.next()).await {}
    console(&td, route, "version").await;
    // A state frame after the line proves no console frame came before it.
    td.plug_device("mock0002").await;
    let next = frame(&mut socket, Duration::from_secs(2), |f| {
        f["type"] == "device.console" || f["type"] == "device.added"
    })
    .await;
    assert_eq!(next["type"], "device.added");
}

#[tokio::test]
async fn key_reports_reach_the_sink_unless_a_dialog_suppresses_them() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    assert!(td.device(&id).unwrap().key_mirror.enabled);
    let mut socket = td.events().await;
    topic(&mut socket, "subscribe", "key-mirror", "mock0001").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let deck = td.deck("mock0001");
    deck.emit_key(&json!({"type": "kbd", "mods": 0, "keys": [4]}));
    deck.emit_key(&json!({"type": "kbd", "mods": 0, "keys": []}));
    td.until(|td| td.input_log() == ["down A", "up A"]).await;
    let typed = frame(&mut socket, Duration::from_secs(2), |f| {
        f["type"] == "device.key-mirror"
    })
    .await;
    assert_eq!(typed["data"]["outcome"], "typed");
    {
        let _dialog = td.dialog();
        deck.emit_key(&json!({"type": "kbd", "mods": 0, "keys": [5]}));
        let suppressed = frame(&mut socket, Duration::from_secs(2), |f| {
            f["type"] == "device.key-mirror" && f["data"]["outcome"] != "typed"
        })
        .await;
        assert_eq!(suppressed["data"]["outcome"], "suppressed");
    }
    // The suppressed report is dropped, never replayed.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(td.input_log(), ["down A", "up A"]);
    deck.emit_key(&json!({"type": "kbd", "mods": 0, "keys": [6]}));
    td.until(|td| td.input_log().last().is_some_and(|k| k == "down C"))
        .await;
    assert!(!td.input_log().iter().any(|k| k.ends_with(" B")));
}

#[tokio::test]
async fn opt_in_frames_need_a_subscription_for_that_device() {
    let td = TestCompanion::start().await;
    td.plug_device("mock0001").await;
    td.plug_device("mock0002").await;
    let mut socket = td.events().await;
    topic(&mut socket, "subscribe", "device-events", "mock0002").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    td.deck("mock0001").emit_press("home", "one", "launch");
    td.deck("mock0002").emit_press("home", "two", "launch");
    let event = frame(&mut socket, Duration::from_secs(2), |f| {
        f["type"] == "device.event"
    })
    .await;
    assert_eq!(event["device"], "mock0002");
    assert_eq!(event["data"]["event"]["tile"], "two");
}

#[tokio::test]
async fn mock_summary_carries_hello_diagnostics() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    let summary = td.call::<Get>(path(&id), NoQuery, ()).await.unwrap();
    let value = serde_json::to_value(summary).unwrap();
    assert_eq!(
        value["diagnostics"],
        json!({"product":"touchdeck", "running_from":"ota_0", "sinks":["log","link"], "current_layout":"builtin"})
    );
}
