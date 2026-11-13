//! Wi-Fi setup through the API, against mock Devices: the network a radio build joins, set over USB. A normal build
//! needs Firmware, a Device on Wi-Fi takes it only over USB, and the password never comes back. Pairing over that
//! network is tests/wifi.rs. Every network and password here is made up.
use serde_json::{Value, json};
use td_companion::{
    api::NoQuery,
    devices::DeviceId,
    features::devices::{
        DevicePath, DeviceWifi, WifiConnection, WifiForget, WifiGet, WifiSecret, WifiSet,
        WifiSetBody,
    },
    testing::TestCompanion,
};

const NETWORK: &str = "td-test-network";
const PASSWORD: &str = "dummy-pass-1234";
const MISSING: &str = "td-test-nonexistent";

fn path(id: &DeviceId) -> DevicePath {
    DevicePath { device: id.clone() }
}

fn body(ssid: &str, password: &str) -> WifiSetBody {
    WifiSetBody {
        ssid: ssid.into(),
        password: WifiSecret::new(password),
    }
}

/// Sends one raw request and returns the status and the body text, so a test can search the text.
async fn raw(
    td: &TestCompanion,
    method: reqwest::Method,
    device: &str,
    body: Option<Value>,
) -> (u16, String) {
    let mut request = td
        .raw()
        .request(method, &format!("/api/v1/devices/{device}/wifi"))
        .bearer_auth(&td.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    (response.status().as_u16(), response.text().await.unwrap())
}

#[tokio::test]
async fn a_radio_device_sets_joins_and_forgets_its_network() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    let device = td.device(&id).unwrap();
    assert_eq!(device.capabilities.unwrap().wifi, Some(true));
    // Its hello says `radio`, so onboarding offers it Wi-Fi setup.
    assert_eq!(
        device.firmware.unwrap().channel,
        td_companion::devices::FirmwareChannel::Radio
    );
    td.deck("mock0001").add_wifi_network(NETWORK, PASSWORD);

    let off = td.call::<WifiGet>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(
        off,
        DeviceWifi {
            configured: false,
            ssid: String::new(),
            state: WifiConnection::Off,
            reason: None,
            ip: None,
            rssi: None,
        }
    );

    let set = td
        .call::<WifiSet>(path(&id), NoQuery, body(MISSING, PASSWORD))
        .await
        .unwrap();
    assert_eq!(
        (set.state, set.ssid.as_str()),
        (WifiConnection::Connecting, MISSING)
    );
    let failed = td.call::<WifiGet>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(failed.state, WifiConnection::Failed);
    assert_eq!(failed.reason.as_deref(), Some("not-found"));
    assert_eq!(failed.ip, None);

    td.call::<WifiSet>(path(&id), NoQuery, body(NETWORK, "wrong-pass-5678"))
        .await
        .unwrap();
    let wrong = td.call::<WifiGet>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(wrong.reason.as_deref(), Some("wrong-password"));

    td.call::<WifiSet>(path(&id), NoQuery, body(NETWORK, PASSWORD))
        .await
        .unwrap();
    let joined = td.call::<WifiGet>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(joined.state, WifiConnection::Connected);
    assert_eq!(joined.ip.as_deref(), Some("192.0.2.10"));
    assert_eq!(joined.rssi, Some(-52));
    assert_eq!(
        td.deck("mock0001").wifi_network(),
        Some((NETWORK.into(), PASSWORD.into()))
    );

    let forgot = td.call::<WifiForget>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(forgot.state, WifiConnection::Off);
    assert_eq!(td.deck("mock0001").wifi_network(), None);
}

#[tokio::test]
async fn an_open_network_needs_no_password() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    td.deck("mock0001").add_wifi_network("td-test-open", "");
    td.call::<WifiSet>(path(&id), NoQuery, body("td-test-open", ""))
        .await
        .unwrap();
    let joined = td.call::<WifiGet>(path(&id), NoQuery, ()).await.unwrap();
    assert_eq!(joined.state, WifiConnection::Connected);
}

#[tokio::test]
async fn a_normal_build_needs_firmware_and_hears_nothing() {
    let td = TestCompanion::start().await;
    let id = td.plug_device("mock0001").await;
    assert_eq!(td.device(&id).unwrap().capabilities.unwrap().wifi, None);
    let before = td.deck("mock0001").received().len();
    for (method, body) in [
        (reqwest::Method::GET, None),
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": NETWORK, "password": PASSWORD})),
        ),
        (reqwest::Method::DELETE, None),
    ] {
        let (status, text) = raw(&td, method, "mock0001", body).await;
        assert_eq!(status, 409, "{text}");
        let error: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(error["error"]["code"], "device.needs_firmware");
        assert_eq!(error["error"]["details"]["field"], "wifi");
        assert!(!text.contains(PASSWORD));
    }
    let received = td.deck("mock0001").received();
    assert!(
        received[before..]
            .iter()
            .all(|r| !r["m"].as_str().unwrap().starts_with("wifi."))
    );
}

#[tokio::test]
async fn invalid_bodies_are_refused_without_echoing_them() {
    let td = TestCompanion::start().await;
    td.plug_radio_device("mock0001").await;
    let before = td.deck("mock0001").received().len();
    let bodies = [
        json!({"ssid": NETWORK, "password": "short"}),
        json!({"ssid": "", "password": PASSWORD}),
        json!({"ssid": "x".repeat(33), "password": PASSWORD}),
        json!({"ssid": "line\nbreak", "password": PASSWORD}),
        json!({"ssid": NETWORK, "password": "g".repeat(64)}),
        json!({"ssid": NETWORK, "password": "café-dummy-pass-1234"}),
        json!({"ssid": NETWORK}),
        json!({"ssid": NETWORK, "password": PASSWORD, "hidden": true}),
        json!({"ssid": 7, "password": PASSWORD}),
        json!({"ssid": NETWORK, "password": [PASSWORD]}),
        json!(PASSWORD),
    ];
    for body in bodies {
        let (status, text) = raw(&td, reqwest::Method::PUT, "mock0001", Some(body.clone())).await;
        assert_eq!(status, 400, "{body}: {text}");
        let error: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(error["error"]["code"], "request.invalid", "{body}");
        assert!(!text.contains(PASSWORD), "{body} came back: {text}");
    }
    assert_eq!(td.deck("mock0001").received().len(), before);
    assert_eq!(td.deck("mock0001").wifi_network(), None);
}

#[tokio::test]
async fn no_response_or_event_carries_the_password() {
    use futures_util::StreamExt;
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    td.deck("mock0001").add_wifi_network(NETWORK, PASSWORD);
    let mut socket = td.events().await;
    let frames = tokio::spawn(async move {
        let mut frames = Vec::new();
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(std::time::Duration::from_millis(500), socket.next()).await
        {
            if let Ok(text) = message.to_text() {
                frames.push(text.to_owned());
            }
        }
        frames
    });
    let mut texts = Vec::new();
    for (method, body) in [
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": NETWORK, "password": PASSWORD})),
        ),
        (reqwest::Method::GET, None),
        (reqwest::Method::GET, None),
        (reqwest::Method::DELETE, None),
    ] {
        let (status, text) = raw(&td, method, id.as_str(), body).await;
        assert_eq!(status, 200, "{text}");
        texts.push(text);
    }
    let devices = td
        .raw()
        .get("/api/v1/devices")
        .bearer_auth(&td.token)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    texts.push(devices);
    texts.extend(frames.await.unwrap());
    let received = serde_json::to_string(&td.deck("mock0001").received()).unwrap();
    texts.push(received);
    for text in &texts {
        assert!(!text.contains(PASSWORD), "the password leaked: {text}");
        assert!(
            !text.contains("\"password\":\"d"),
            "a password field leaked: {text}"
        );
    }
    let debug = format!("{:?}", body(NETWORK, PASSWORD));
    assert!(!debug.contains(PASSWORD), "{debug}");
}

#[tokio::test]
async fn a_device_that_refuses_off_usb_stores_nothing() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    td.deck("mock0001").set_wifi_usb_only(true);
    let (status, text) = raw(
        &td,
        reqwest::Method::PUT,
        id.as_str(),
        Some(json!({"ssid": NETWORK, "password": PASSWORD})),
    )
    .await;
    assert_eq!(status, 409, "{text}");
    let error: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(error["error"]["code"], "device.usb_only");
    assert!(!text.contains(PASSWORD));
    assert_eq!(td.deck("mock0001").wifi_network(), None);
}

#[tokio::test]
async fn a_device_on_wifi_hears_no_setup() {
    // Mock Devices report the id 020000000001, so USB and Wi-Fi merge under that serial.
    const SERIAL: &str = "020000000001";
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    td.deck(SERIAL).set_wifi_capability(true);
    td.deck(SERIAL).add_wifi_network(NETWORK, PASSWORD);
    let id = td.pair_wifi(&fake).await;
    assert_eq!(
        td.device(&id).unwrap().capabilities.unwrap().wifi,
        Some(true)
    );
    let before = td.deck(SERIAL).received().len();
    for (method, body) in [
        (reqwest::Method::GET, None),
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": NETWORK, "password": PASSWORD})),
        ),
        (reqwest::Method::DELETE, None),
    ] {
        let (status, text) = raw(&td, method, id.as_str(), body).await;
        assert_eq!(status, 409, "{text}");
        let error: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(error["error"]["code"], "device.usb_only");
        assert!(!text.contains(PASSWORD));
    }
    let received = td.deck(SERIAL).received();
    assert!(
        received[before..]
            .iter()
            .all(|r| !r["m"].as_str().unwrap().starts_with("wifi."))
    );
    assert_eq!(td.deck(SERIAL).wifi_network(), None);
}

#[tokio::test]
async fn an_unknown_device_is_not_found() {
    let td = TestCompanion::start().await;
    let (status, _) = raw(&td, reqwest::Method::GET, "mock9999", None).await;
    assert_eq!(status, 404);
}

/// Set by the log check below when it runs this test binary again as its child.
const LOG_CHILD: &str = "TOUCHDECK_WIFI_LOG_CHILD";
/// A password the log check uses only here, so no other test's output can carry it.
const LOG_PASSWORD: &str = "dummy-pass-7319";

/// A Wi-Fi password set through the API never reaches the logs. td-app runs this code in its own
/// process and logs to stderr, so the check runs this test binary again with `--nocapture`: the
/// child drives the Wi-Fi requests, and this test reads everything the child wrote to stdout and
/// stderr, from every thread.
#[tokio::test]
async fn a_wifi_password_never_reaches_the_logs() {
    if std::env::var_os(LOG_CHILD).is_some() {
        return drive_wifi_for_the_log_check().await;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "a_wifi_password_never_reaches_the_logs",
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(LOG_CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("1 passed"),
        "the child ran the check: {stdout}"
    );
    assert!(
        stderr.contains("log check: a worker thread's stderr is captured"),
        "the child's stderr from the runtime threads was captured: {stderr}"
    );
    for text in [&stdout, &stderr] {
        assert!(
            !text.contains(LOG_PASSWORD),
            "the Wi-Fi password leaked: {text}"
        );
    }
}

/// The child's half of the log check: set, read, reject and delete a password on a mock Device.
async fn drive_wifi_for_the_log_check() {
    tokio::spawn(async { eprintln!("log check: a worker thread's stderr is captured") })
        .await
        .unwrap();
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device("mock0001").await;
    td.deck("mock0001").add_wifi_network(NETWORK, LOG_PASSWORD);
    for (method, body, expected) in [
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": MISSING, "password": LOG_PASSWORD})),
            200,
        ),
        (reqwest::Method::GET, None, 200),
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": NETWORK, "password": LOG_PASSWORD})),
            200,
        ),
        (reqwest::Method::GET, None, 200),
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": NETWORK, "password": LOG_PASSWORD, "extra": LOG_PASSWORD})),
            400,
        ),
        (
            reqwest::Method::PUT,
            Some(json!({"ssid": "", "password": LOG_PASSWORD})),
            400,
        ),
        (reqwest::Method::DELETE, None, 200),
    ] {
        let (status, text) = raw(&td, method, id.as_str(), body).await;
        assert_eq!(status, expected, "{text}");
    }
    assert_eq!(td.deck("mock0001").wifi_network(), None);
}
