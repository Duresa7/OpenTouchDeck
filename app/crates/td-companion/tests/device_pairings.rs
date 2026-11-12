//! The apps a radio-build Device keeps Pairings with, through the API against mock Devices: the list over USB, forget
//! one or every one, this Companion's own key going with its Pairing, and a forget whose answer never came, sent
//! again once the Device is back. A normal build needs Firmware, and a Device linked over Wi-Fi hears nothing.
use serde_json::Value;
use std::time::Duration;
use td_companion::{
    api::NoQuery,
    devices::{Carrier, DeviceId},
    features::{
        devices::DevicePath,
        pairings::{AppPath, PairingsForget, PairingsForgetAll, PairingsGet},
        wifi::PairingsList,
    },
    testing::TestCompanion,
};
use td_link::net::fake::FakeNetDevice;

const SERIAL: &str = "020000000001";
const OTHER: [u8; 16] = [0x20; 16];

fn path(id: &DeviceId) -> DevicePath {
    DevicePath { device: id.clone() }
}

fn app(id: &DeviceId, app: &str) -> AppPath {
    AppPath {
        device: id.clone(),
        app: app.into(),
    }
}

fn hex(id: &[u8; 16]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

async fn error_code(td: &TestCompanion, method: reqwest::Method, route: &str) -> (u16, Value) {
    let response = td
        .raw()
        .request(method, &format!("/api/v1{route}"))
        .bearer_auth(&td.token)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    (status, body["error"]["code"].clone())
}

fn forgets(td: &TestCompanion) -> usize {
    td.deck(SERIAL)
        .received()
        .iter()
        .filter(|r| r["m"] == "net.forget")
        .count()
}

/// Pairs over Wi-Fi, then plugs the same Device in over USB, which wins.
/// The fake Device must outlive the test: its store holds the Pairing.
async fn paired_then_plugged(td: &TestCompanion) -> (DeviceId, FakeNetDevice) {
    let fake = td.wifi_radio_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    td.plug_radio_device(SERIAL).await;
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| d.carrier == Some(Carrier::Usb))
    })
    .await;
    (id, fake)
}

#[tokio::test]
async fn usb_lists_the_apps_and_forgets_one_then_all() {
    let td = TestCompanion::start().await;
    let (id, _fake) = paired_then_plugged(&td).await;

    let listed = td
        .call::<PairingsGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(listed.max, 4);
    assert_eq!(listed.pairings.len(), 1);
    let own = listed.pairings[0].clone();
    assert!(own.this_app);
    assert_eq!(own.app.len(), 32);

    // Forgetting this Companion on the Device also drops its key here.
    let after = td
        .call::<PairingsForget>(app(&id, &own.app), NoQuery, ())
        .await
        .unwrap();
    assert!(after.pairings.is_empty());
    assert!(!td.wifi_keys().has(&id));
    let held = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
    assert!(held.is_empty());

    // Forgetting an app the Device doesn't keep succeeds and removes none.
    td.call::<PairingsForget>(app(&id, &own.app), NoQuery, ())
        .await
        .unwrap();
    td.call::<PairingsForgetAll>(path(&id), NoQuery, ())
        .await
        .unwrap();
}

#[tokio::test]
async fn the_local_list_names_the_other_apps() {
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device(SERIAL).await;
    let deck = td.deck(SERIAL);
    deck.add_net_pairing([0x10; 16], "Desk PC");
    deck.add_net_pairing(OTHER, "Laptop");
    let listed = td
        .call::<PairingsGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    let names: Vec<&str> = listed.pairings.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Desk PC", "Laptop"]);
    assert!(listed.pairings.iter().all(|p| !p.this_app));

    let after = td
        .call::<PairingsForget>(app(&id, &hex(&OTHER)), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(after.pairings.len(), 1);
    let none = td
        .call::<PairingsForgetAll>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert!(none.pairings.is_empty());
    assert!(deck.net_pairings().is_empty());
}

#[tokio::test]
async fn an_unanswered_forget_keeps_the_key_marked_until_the_device_is_back() {
    let td = TestCompanion::start().await;
    let (id, _fake) = paired_then_plugged(&td).await;
    let own = td
        .call::<PairingsGet>(path(&id), NoQuery, ())
        .await
        .unwrap()
        .pairings[0]
        .app
        .clone();

    // The Device forgets, but its answer is lost.
    td.deck(SERIAL).lose_net_forget_replies(1);
    let (status, code) = error_code(
        &td,
        reqwest::Method::DELETE,
        &format!("/devices/{id}/pairings/{own}"),
    )
    .await;
    assert!(
        ["device.timeout", "device.unknown_outcome"].contains(&code.as_str().unwrap_or_default()),
        "{status} {code}"
    );
    assert!(
        td.wifi_keys().has(&id),
        "the key stays until the Device answers"
    );
    let held = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
    assert!(held[0].removing);
    let sent = forgets(&td);

    // Plugged in again, the Companion sends the forget again and drops the key.
    td.unplug(SERIAL).await;
    td.plug_radio_device(SERIAL).await;
    td.until(|td| !td.wifi_keys().has(&id)).await;
    assert!(forgets(&td) > sent);
    let held = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
    assert!(held.is_empty());
}

#[tokio::test]
async fn a_normal_build_needs_firmware_and_a_bad_app_id_is_refused() {
    let td = TestCompanion::start().await;
    let id = td.plug_device(SERIAL).await;
    let route = format!("/devices/{id}/pairings");
    assert_eq!(
        error_code(&td, reqwest::Method::GET, &route).await,
        (409, "device.needs_firmware".into())
    );
    let td = TestCompanion::start().await;
    let id = td.plug_radio_device(SERIAL).await;
    for bad in ["1011", &"AB".repeat(16), &"g0".repeat(16)] {
        let route = format!("/devices/{id}/pairings/{bad}");
        assert_eq!(
            error_code(&td, reqwest::Method::DELETE, &route).await,
            (400, "request.invalid".into()),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn over_wifi_the_device_hears_nothing() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_radio_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    let before = fake.deck().received().len();
    for (method, route) in [
        (reqwest::Method::GET, format!("/devices/{id}/pairings")),
        (reqwest::Method::DELETE, format!("/devices/{id}/pairings")),
        (
            reqwest::Method::DELETE,
            format!("/devices/{id}/pairings/{}", hex(&OTHER)),
        ),
    ] {
        assert_eq!(
            error_code(&td, method, &route).await,
            (409, "device.usb_only".into())
        );
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        fake.deck().received()[before..]
            .iter()
            .all(|r| !r["m"].as_str().unwrap_or_default().starts_with("net.")),
    );
    assert_eq!(fake.paired(), 1);
}

#[tokio::test]
async fn a_refused_forget_keeps_the_mark_and_key_until_a_successful_retry() {
    let td = TestCompanion::start().await;
    let (id, _fake) = paired_then_plugged(&td).await;
    let own = td
        .call::<PairingsGet>(path(&id), NoQuery, ())
        .await
        .unwrap()
        .pairings[0]
        .app
        .clone();
    let deck = td.deck(SERIAL);
    deck.fail_net_storage(true);
    assert_eq!(
        error_code(
            &td,
            reqwest::Method::DELETE,
            &format!("/devices/{id}/pairings/{own}")
        )
        .await,
        (502, "pairings.storage".into())
    );
    assert!(td.wifi_keys().has(&id));
    assert!(td.call::<PairingsList>((), NoQuery, ()).await.unwrap()[0].removing);
    let sent = forgets(&td);
    // A refused retry is attempted once; listing still completes and keeps the mark and key.
    let listed = td
        .call::<PairingsGet>(path(&id), NoQuery, ())
        .await
        .unwrap();
    assert_eq!(listed.pairings.len(), 1);
    assert_eq!(forgets(&td), sent + 1);
    assert!(td.wifi_keys().has(&id));
    assert!(td.call::<PairingsList>((), NoQuery, ()).await.unwrap()[0].removing);
    deck.fail_net_storage(false);
    assert!(
        td.call::<PairingsGet>(path(&id), NoQuery, ())
            .await
            .unwrap()
            .pairings
            .is_empty()
    );
    assert!(!td.wifi_keys().has(&id));
    assert!(
        td.call::<PairingsList>((), NoQuery, ())
            .await
            .unwrap()
            .is_empty()
    );
}
