//! The Bluetooth keyboard through the API, against mock Devices: a radio build turns it on, opens the pairing window,
//! lists the computers and phones it keeps and forgets them, over USB only. A normal build needs Firmware, and a Device
//! linked over Wi-Fi hears nothing. The addresses are documentation addresses (RFC 7042).
use serde_json::{Value, json};
use td_companion::{
    api::NoQuery,
    devices::DeviceId,
    features::{
        bluetooth::{
            BluetoothAdvertising, BluetoothForget, BluetoothForgetAll, BluetoothGet,
            BluetoothPairingClose, BluetoothPairingOpen, BluetoothSet, BluetoothSetBody,
            BluetoothStack, BondPath,
        },
        devices::DevicePath,
    },
    testing::TestCompanion,
};

const PHONE: &str = "00:00:5e:00:53:01";
const LAPTOP: &str = "00:00:5e:00:53:02";

fn path(id: &DeviceId) -> DevicePath {
    DevicePath { device: id.clone() }
}

fn on(enabled: bool) -> BluetoothSetBody {
    BluetoothSetBody { enabled }
}

/// Sends one raw request and returns the status and the parsed body.
async fn raw(
    td: &TestCompanion,
    method: reqwest::Method,
    route: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut request = td
        .raw()
        .request(method, &format!("/api/v1{route}"))
        .bearer_auth(&td.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Every route, as `(method, route, body)`, for the Device `id`.
fn routes(id: &str) -> Vec<(reqwest::Method, String, Option<Value>)> {
    let base = format!("/devices/{id}/bluetooth");
    vec![
        (reqwest::Method::GET, base.clone(), None),
        (
            reqwest::Method::PUT,
            base.clone(),
            Some(json!({"enabled": true})),
        ),
        (reqwest::Method::POST, format!("{base}/pairing"), None),
        (reqwest::Method::DELETE, format!("{base}/pairing"), None),
        (
            reqwest::Method::DELETE,
            format!("{base}/bonds/{PHONE}"),
            None,
        ),
        (reqwest::Method::DELETE, format!("{base}/bonds"), None),
    ]
}

fn ble_requests(td: &TestCompanion, serial: &str, from: usize) -> usize {
    td.deck(serial).received()[from..]
        .iter()
        .filter(|r| r["m"].as_str().unwrap_or_default().starts_with("ble."))
        .count()
}

#[tokio::test]
async fn a_radio_device_turns_on_pairs_and_turns_off() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    assert_eq!(
        td.device(&id).unwrap().capabilities.unwrap().ble_hid,
        Some(4)
    );

    let off = td
        .call::<BluetoothGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert!(!off.enabled, "off on a fresh Device");
    assert_eq!(off.state, BluetoothStack::Off);
    assert_eq!((off.bonds_max, off.battery), (4, 100));

    let started = td
        .call::<BluetoothSet>(path(&id), NoQuery, on(true))
        .await
        .unwrap();
    // As the Firmware: a start answers `starting`, and the host has synced by the next request.
    assert_eq!(
        (started.enabled, started.state),
        (true, BluetoothStack::Starting)
    );
    assert!(td.deck("mock0001").ble_enabled());
    let synced = td
        .call::<BluetoothGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(synced.state, BluetoothStack::On);

    let open = td
        .call::<BluetoothPairingOpen>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert!(open.pairing);
    assert!(open.pairing_ms > 110_000 && open.pairing_ms <= 120_000);
    assert_eq!(open.advertising, BluetoothAdvertising::Pairing);

    td.deck("mock0001").ble_show_passkey(true);
    let showing = td
        .call::<BluetoothGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert!(showing.passkey);

    assert!(td.deck("mock0001").ble_host_pairs(PHONE, false));
    let paired = td
        .call::<BluetoothGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert!(!paired.pairing && !paired.passkey);
    assert_eq!(paired.bonds.len(), 1);
    assert_eq!(paired.bonds[0].address, PHONE);
    assert!(paired.bonds[0].connected);

    td.call::<BluetoothPairingOpen>(path(&id), NoQuery, ())
        .await
        .unwrap();
    let closed = td
        .call::<BluetoothPairingClose>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!((closed.pairing, closed.pairing_ms), (false, 0));

    let off = td
        .call::<BluetoothSet>(path(&id), NoQuery, on(false))
        .await
        .unwrap();
    assert_eq!(off.state, BluetoothStack::Off);
    assert!(off.bonds.is_empty(), "a Device that is off lists no bonds");
    assert_eq!(td.deck("mock0001").ble_bonds().len(), 1);
}

#[tokio::test]
async fn forgets_one_host_then_every_one() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    let deck = td.deck("mock0001");
    deck.add_ble_bond(PHONE, false, true);
    deck.add_ble_bond(LAPTOP, true, false);
    td.call::<BluetoothSet>(path(&id), NoQuery, on(true))
        .await
        .unwrap();

    // Either case reaches the Device in lower case.
    let one = td
        .call::<BluetoothForget>(
            BondPath {
                device: id.clone(),
                address: PHONE.to_ascii_uppercase(),
            },
            NoQuery,
            (),
        )
        .await
        .unwrap();
    assert_eq!(one.forgotten, 1);
    assert_eq!(one.bluetooth.bonds.len(), 1);
    assert_eq!(one.bluetooth.bonds[0].address, LAPTOP);
    assert!(one.bluetooth.bonds[0].random);
    let sent = deck.received();
    assert_eq!(
        sent.iter().rev().find(|r| r["m"] == "ble.forget").unwrap()["address"],
        PHONE
    );

    let all = td
        .call::<BluetoothForgetAll>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(all.forgotten, 1);
    assert!(all.bluetooth.bonds.is_empty());
    assert!(deck.ble_bonds().is_empty());
}

#[tokio::test]
async fn device_refusals_name_the_reason_and_carry_the_state() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    let base = format!("/devices/{id}/bluetooth", id = id.as_str());

    let (status, body) = raw(&td, reqwest::Method::POST, &format!("{base}/pairing"), None).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "bluetooth.off");
    assert_eq!(body["error"]["details"]["bluetooth"]["state"], "off");
    let (status, body) = raw(&td, reqwest::Method::DELETE, &format!("{base}/bonds"), None).await;
    assert_eq!(
        (status, &body["error"]["code"]),
        (409, &json!("bluetooth.off"))
    );

    let deck = td.deck("mock0001");
    for n in 1..=4 {
        deck.add_ble_bond(&format!("00:00:5e:00:53:0{n}"), false, false);
    }
    td.call::<BluetoothSet>(path(&id), NoQuery, on(true))
        .await
        .unwrap();
    let (status, body) = raw(&td, reqwest::Method::POST, &format!("{base}/pairing"), None).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "bluetooth.full");
    assert_eq!(
        body["error"]["details"]["bluetooth"]["bonds"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    deck.fail_ble_start(true);
    td.call::<BluetoothSet>(path(&id), NoQuery, on(false))
        .await
        .unwrap();
    let (status, body) = raw(
        &td,
        reqwest::Method::PUT,
        &base,
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(status, 502, "{body}");
    assert_eq!(body["error"]["code"], "bluetooth.radio");
    assert_eq!(body["error"]["details"]["bluetooth"]["enabled"], true);
    assert_eq!(body["error"]["details"]["bluetooth"]["state"], "failed");
}

#[tokio::test]
async fn invalid_requests_are_refused_before_the_device_hears_them() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    let base = format!("/devices/{id}/bluetooth", id = id.as_str());
    let before = td.deck("mock0001").received().len();
    for body in [
        json!({}),
        json!({"enabled": "yes"}),
        json!({"enabled": true, "extra": 1}),
    ] {
        let (status, error) = raw(&td, reqwest::Method::PUT, &base, Some(body.clone())).await;
        assert_eq!(status, 400, "{body}: {error}");
        assert_eq!(error["error"]["code"], "request.invalid");
    }
    for address in ["00-00-5e-00-53-01", "00:00:5e:00:53", "zz:00:5e:00:53:01"] {
        let (status, error) = raw(
            &td,
            reqwest::Method::DELETE,
            &format!("{base}/bonds/{address}"),
            None,
        )
        .await;
        assert_eq!(status, 400, "{address}: {error}");
        assert_eq!(error["error"]["code"], "request.invalid");
    }
    assert_eq!(ble_requests(&td, "mock0001", before), 0);
}

#[tokio::test]
async fn a_normal_build_needs_firmware_and_hears_nothing() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    assert_eq!(td.device(&id).unwrap().capabilities.unwrap().ble_hid, None);
    let before = td.deck("mock0001").received().len();
    for (method, route, body) in routes(id.as_str()) {
        let (status, error) = raw(&td, method, &route, body).await;
        assert_eq!(status, 409, "{route}: {error}");
        assert_eq!(error["error"]["code"], "device.needs_firmware");
        assert_eq!(error["error"]["details"]["field"], "bluetooth");
    }
    assert_eq!(ble_requests(&td, "mock0001", before), 0);
}

#[tokio::test]
async fn a_device_on_wifi_hears_no_bluetooth_request() {
    // Mock Devices report the id 020000000001, so USB and Wi-Fi merge under that serial.
    const SERIAL: &str = "020000000001";
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    td.deck(SERIAL).set_ble_capability(Some(4));
    td.deck(SERIAL).add_ble_bond(PHONE, false, false);
    let id = td.pair_wifi(&fake).await;
    assert_eq!(
        td.device(&id).unwrap().capabilities.unwrap().ble_hid,
        Some(4)
    );
    let before = td.deck(SERIAL).received().len();
    for (method, route, body) in routes(id.as_str()) {
        let (status, error) = raw(&td, method, &route, body).await;
        assert_eq!(status, 409, "{route}: {error}");
        assert_eq!(error["error"]["code"], "device.usb_only");
    }
    assert_eq!(ble_requests(&td, SERIAL, before), 0);
    assert!(!td.deck(SERIAL).ble_enabled());
    assert_eq!(td.deck(SERIAL).ble_bonds().len(), 1);
}

#[tokio::test]
async fn the_device_itself_refuses_off_usb() {
    // The Companion checks the Carrier first. Should a request still reach the Device over the network, the mock
    // refuses it `usb-only`, as the Firmware does, and the Companion maps that to the same error.
    let error =
        td_companion::features::bluetooth::BluetoothError::from(td_link::ble::BleError::Refused {
            reason: "usb-only".into(),
            error: "Only over USB".into(),
            state: None,
        });
    use td_companion::api::ApiError;
    assert_eq!(
        (error.status().as_u16(), error.code()),
        (409, "device.usb_only")
    );
}

#[tokio::test]
async fn an_unknown_device_is_not_found() {
    let td = TestCompanion::start().await;
    let (status, _) = raw(
        &td,
        reqwest::Method::GET,
        "/devices/mock9999/bluetooth",
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn while_starting_the_window_and_forget_wait_and_the_bonds_stay_listed() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    let base = format!("/devices/{id}/bluetooth", id = id.as_str());
    let deck = td.deck("mock0001");
    deck.add_ble_bond(PHONE, false, true);
    deck.ble_hold_start(true);
    let started = td
        .call::<BluetoothSet>(path(&id), NoQuery, on(true))
        .await
        .unwrap();
    assert_eq!(started.state, BluetoothStack::Starting);
    for (method, route) in [
        (reqwest::Method::POST, format!("{base}/pairing")),
        (reqwest::Method::DELETE, format!("{base}/bonds/{PHONE}")),
    ] {
        let (status, body) = raw(&td, method, &route, None).await;
        assert_eq!(status, 409, "{route}: {body}");
        assert_eq!(body["error"]["code"], "bluetooth.starting");
        assert_eq!(body["error"]["details"]["bluetooth"]["state"], "starting");
    }

    // After a host reset the Device says `starting` again and still lists its bonds.
    deck.ble_hold_start(false);
    assert_eq!(
        td.call::<BluetoothGet>(path(&id), NoQuery, ())
            .await
            .unwrap()
            .bonds
            .len(),
        1
    );
    deck.ble_hold_start(true);
    deck.ble_host_reset();
    let reset = td
        .call::<BluetoothGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(reset.state, BluetoothStack::Starting);
    assert_eq!(reset.bonds.len(), 1);
    assert_eq!(reset.bonds[0].address, PHONE);
    deck.ble_hold_start(false);
    assert_eq!(
        td.call::<BluetoothGet>(path(&id), NoQuery, ())
            .await
            .unwrap()
            .state,
        BluetoothStack::On
    );
}

#[tokio::test]
async fn a_storage_failure_says_so_and_changes_nothing() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    let base = format!("/devices/{id}/bluetooth", id = id.as_str());
    let deck = td.deck("mock0001");
    deck.add_ble_bond(PHONE, false, false);
    deck.fail_ble_storage(true);
    let (status, body) = raw(
        &td,
        reqwest::Method::PUT,
        &base,
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(status, 502, "{body}");
    assert_eq!(body["error"]["code"], "bluetooth.storage");
    assert_eq!(body["error"]["details"]["bluetooth"]["enabled"], false);
    assert!(!deck.ble_enabled());

    deck.fail_ble_storage(false);
    td.call::<BluetoothSet>(path(&id), NoQuery, on(true))
        .await
        .unwrap();
    td.call::<BluetoothGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    deck.fail_ble_storage(true);
    let (status, body) = raw(
        &td,
        reqwest::Method::DELETE,
        &format!("{base}/bonds/{PHONE}"),
        None,
    )
    .await;
    assert_eq!(status, 502, "{body}");
    assert_eq!(body["error"]["code"], "bluetooth.storage");
    assert_eq!(
        body["error"]["details"]["bluetooth"]["bonds"][0]["address"],
        PHONE
    );
    assert_eq!(deck.ble_bonds().len(), 1);
}

#[tokio::test]
async fn an_unknown_ble_message_on_a_radio_device_needs_firmware() {
    // The Companion sends only the four messages it knows. Should a radio build answer one of them `unsupported`,
    // as it answers a `ble.*` message it doesn't know, the Companion says the Device needs Firmware.
    let td = TestCompanion::start().await;
    td.plug_radio_device("mock0001").await;
    let session_reply = td.deck("mock0001").received().len();
    let error =
        td_companion::features::bluetooth::BluetoothError::from(td_link::ble::BleError::Refused {
            reason: "unsupported".into(),
            error: "Unknown Bluetooth message".into(),
            state: None,
        });
    use td_companion::api::ApiError;
    assert_eq!(
        (error.status().as_u16(), error.code()),
        (409, "device.needs_firmware")
    );
    assert_eq!(ble_requests(&td, "mock0001", session_reply), 0);
}

#[tokio::test]
async fn unplugging_in_the_middle_of_a_request_fails_it_and_a_replug_works() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    td.deck("mock0001")
        .delay_replies(std::time::Duration::from_millis(1500));
    let request = tokio::spawn(
        td.request::<BluetoothSet>(path(&id), NoQuery, &on(true))
            .send(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    td.unplug("mock0001").await;
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), request)
        .await
        .expect("the request ends once the Device is gone")
        .unwrap()
        .unwrap();
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    assert!(
        [409, 502, 504].contains(&status),
        "an unplugged Device fails the request: {status} {body}"
    );
    assert!(
        body["error"]["code"]
            .as_str()
            .is_some_and(|code| code.starts_with("device.")),
        "{body}"
    );

    td.deck("mock0001").delay_replies(std::time::Duration::ZERO);
    let id = td.plug_radio_device("mock0001").await;
    let state = td
        .call::<BluetoothGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(state.bonds_max, 4);
}
