//! The Wi-Fi Carrier against fake net-v2 Devices on loopback: discovery, a typed address,
//! pairing, reconnecting, busy, the USB/Wi-Fi switch, a Layout push and Firmware.
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use td_companion::{
    api::NoQuery,
    devices::{Carrier, LinkState},
    features::{
        devices::{Connect, DevicePath, Disconnect},
        firmware::{FirmwareRequest, FirmwareTarget, Update},
        layouts::{LayoutBody, LayoutPath, Push},
        wifi::{
            AttemptPath, Discover, PairCancel, PairCode, PairCodeEndpoint, PairStart,
            PairStartEndpoint, PairingPath, PairingsForget, PairingsList,
        },
    },
    jobs::JobState,
    testing::TestCompanion,
};
use td_core::{Layout, Orientation};
use td_link::net::{self, NetCarrier, fake::FakeNetDevice};

/// Mock Devices report this id in `hello`, so USB and Wi-Fi merge under it.
const SERIAL: &str = "020000000001";

/// Starts a pairing as another app once the fake Device has let go of the previous connection: right after a
/// pairing it can still answer `busy` for a moment, more often on a loaded CI runner.
fn pair_when_free(address: &net::Address, app: [u8; 16]) -> net::Pairing {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        match net::pair(address, &app, "Other app", Duration::from_secs(5)) {
            Err(net::NetError::Refused(net::Refusal::Busy)) if Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result.unwrap(),
        }
    }
}

/// The status and error body of a request that fails.
async fn failure(request: reqwest::RequestBuilder) -> (u16, Value) {
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    (
        status,
        response.json::<Value>().await.unwrap()["error"].clone(),
    )
}

async fn start(td: &TestCompanion, fake: &FakeNetDevice) -> String {
    fake.open_window().unwrap();
    td.call::<PairStartEndpoint>(
        (),
        NoQuery,
        PairStart {
            address: fake.addr().to_string(),
        },
    )
    .await
    .unwrap()
    .attempt
}

fn code(td: &TestCompanion, attempt: &str, code: &str) -> reqwest::RequestBuilder {
    td.request::<PairCodeEndpoint>(
        AttemptPath {
            attempt: attempt.into(),
        },
        NoQuery,
        &PairCode { code: code.into() },
    )
}

fn wrong(code: &str) -> String {
    let n: u32 = code.parse().unwrap();
    format!("{:06}", (n + 1) % 1_000_000)
}

fn on_wifi(td: &TestCompanion, id: &td_companion::devices::DeviceId) -> bool {
    td.device(id).is_some_and(|d| {
        d.carrier == Some(Carrier::Wifi) && matches!(d.link, LinkState::Connected { .. })
    })
}

#[tokio::test]
async fn discovery_finds_the_device_and_a_typed_address_pairs_it() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, true);
    let found = td.call::<Discover>((), NoQuery, ()).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].device, SERIAL);
    assert_eq!(found[0].address, fake.addr().to_string());
    assert!(!found[0].paired_here);
    assert_eq!(td.wifi_network().browses(), 1);

    // Multicast blocked: discovery finds nothing, and the typed address still works.
    td.wifi_network().withdraw(&fake.device_id());
    assert!(
        td.call::<Discover>((), NoQuery, ())
            .await
            .unwrap()
            .is_empty()
    );
    let id = td.pair_wifi(&fake).await;
    assert_eq!(id, SERIAL);
    let device = td.device(&id).unwrap();
    assert_eq!(
        device.link,
        LinkState::Connected {
            port: fake.addr().to_string(),
            since_ms: match device.link {
                LinkState::Connected { since_ms, .. } => since_ms,
                _ => unreachable!(),
            },
            protocol: 2,
        }
    );
    assert_eq!(device.name.as_deref(), Some("Mock Device 020000000001"));
    let serialized = serde_json::to_value(&device).unwrap();
    assert_eq!(serialized["carrier"], "wifi");

    let pairings = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
    assert_eq!(pairings.len(), 1);
    assert!(pairings[0].confirmed);
    assert_eq!(pairings[0].address, fake.addr().to_string());
    assert!(td.wifi_keys().has(&id), "the key is in the keychain");
    assert_eq!(fake.paired(), 1);
    assert!(fake.paired_names()[0].starts_with("Open TouchDeck on "));

    td.wifi_network().announce(net::mdns::Discovered {
        device_id: fake.device_id(),
        name: "Desk".into(),
        address: fake.addr().into(),
        paired: Some(1),
    });
    let found = td.call::<Discover>((), NoQuery, ()).await.unwrap();
    assert!(found[0].paired_here);
}

#[tokio::test]
async fn a_wrong_code_leaves_attempts_and_the_right_one_pairs() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let attempt = start(&td, &fake).await;
    let right = fake.code().unwrap();
    let (status, error) = failure(code(&td, &attempt, &wrong(&right))).await;
    assert_eq!(status, 422);
    assert_eq!(error["code"], "wifi.wrong_code");
    assert_eq!(error["details"]["attempts_left"], 2);
    let (status, error) = failure(code(&td, &attempt, "12ab56")).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (400, Some("request.invalid"))
    );
    let paired = td
        .call::<PairCodeEndpoint>(
            AttemptPath {
                attempt: attempt.clone(),
            },
            NoQuery,
            PairCode { code: right },
        )
        .await
        .unwrap();
    assert_eq!(paired.device, SERIAL);
    td.until(|td| on_wifi(td, &paired.device)).await;
    let (status, error) = failure(code(&td, &attempt, "000000")).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("wifi.attempt_not_found"))
    );
}

#[tokio::test]
async fn three_wrong_codes_lock_pairing_out() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let attempt = start(&td, &fake).await;
    let right = fake.code().unwrap();
    for _ in 0..2 {
        let (_, error) = failure(code(&td, &attempt, &wrong(&right))).await;
        assert_eq!(error["code"], "wifi.wrong_code");
    }
    let (status, error) = failure(code(&td, &attempt, &wrong(&right))).await;
    assert_eq!((status, error["code"].as_str()), (429, Some("wifi.locked")));
    assert!(fake.locked_for() > Duration::from_secs(25));
    let (status, error) = failure(td.request::<PairStartEndpoint>(
        (),
        NoQuery,
        &PairStart {
            address: fake.addr().to_string(),
        },
    ))
    .await;
    assert_eq!(
        (status, error["code"].as_str()),
        (409, Some("wifi.window_closed"))
    );
    assert!(
        error["message"].as_str().unwrap().contains("Pair new app"),
        "{error}"
    );
    assert!(
        td.call::<PairingsList>((), NoQuery, ())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn pairing_needs_the_window_open_and_a_reachable_address() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let (status, error) = failure(td.request::<PairStartEndpoint>(
        (),
        NoQuery,
        &PairStart {
            address: fake.addr().to_string(),
        },
    ))
    .await;
    assert_eq!(
        (status, error["code"].as_str()),
        (409, Some("wifi.window_closed"))
    );
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let nobody = closed.local_addr().unwrap().to_string();
    drop(closed);
    let (status, error) =
        failure(td.request::<PairStartEndpoint>((), NoQuery, &PairStart { address: nobody })).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (502, Some("wifi.unreachable"))
    );
    let (_, error) = failure(td.request::<PairStartEndpoint>(
        (),
        NoQuery,
        &PairStart {
            address: "not an address".into(),
        },
    ))
    .await;
    assert_eq!(error["code"], "wifi.unreachable");
}

#[tokio::test]
async fn cancel_ends_the_window_on_the_device() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let attempt = start(&td, &fake).await;
    assert!(fake.pairing());
    td.call::<PairCancel>(AttemptPath { attempt }, NoQuery, ())
        .await
        .unwrap();
    td.until(|_| !fake.pairing()).await;
    assert_eq!(fake.paired(), 0);
}

#[tokio::test]
async fn without_a_keychain_pairing_fails_clearly_and_keeps_nothing() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    td.wifi_keys().set_unavailable(true);
    let attempt = start(&td, &fake).await;
    let right = fake.code().unwrap();
    let (status, error) = failure(code(&td, &attempt, &right)).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (503, Some("wifi.keychain"))
    );
    assert!(
        error["message"].as_str().unwrap().contains("pair again"),
        "{error}"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.paired(), 0, "the Device discards the uncommitted key");
    assert!(
        td.call::<PairingsList>((), NoQuery, ())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_dropped_wifi_link_reconnects() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    assert_eq!(fake.sessions(), 1);
    fake.drop_connection();
    td.until(|td| !on_wifi(td, &id) || fake.sessions() > 1)
        .await;
    td.until(|td| on_wifi(td, &id) && fake.sessions() == 2)
        .await;
    assert_eq!(td.device_count(), 1);
}

#[tokio::test]
async fn a_busy_device_shows_busy_until_the_other_app_lets_go() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    td.call::<Disconnect>(DevicePath { device: id.clone() }, NoQuery, ())
        .await
        .unwrap();
    td.until(|_| fake.session().is_none()).await;

    // Another app pairs and holds the Device.
    fake.open_window().unwrap();
    let address = net::Address::from(fake.addr());
    let mut other = net::pair(&address, &[0x77; 16], "Other app", Duration::from_secs(5)).unwrap();
    let paired = other
        .submit(&fake.code().unwrap(), &mut |_| Ok(()))
        .unwrap();
    // The pairing connection holds the Device until it closes, so the first try may be busy.
    let end = std::time::Instant::now() + Duration::from_secs(5);
    let carrier = loop {
        match NetCarrier::connect(&address, &paired.credential, Duration::from_secs(5)) {
            Err(net::NetError::Refused(net::Refusal::Busy)) if std::time::Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(50));
            }
            other => break other.unwrap(),
        }
    };
    let (session, _incoming) = td_link::Session::over(
        Box::new(carrier),
        td_link::SessionConfig {
            ping_every: None,
            ..Default::default()
        },
    )
    .unwrap();

    td.call::<Connect>(DevicePath { device: id.clone() }, NoQuery, ())
        .await
        .unwrap();
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| matches!(d.link, LinkState::Busy { .. }))
    })
    .await;
    let pairings = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
    assert!(pairings[0].problem.as_deref().unwrap().contains("busy"));
    session.close();
    drop(session);
    td.until(|td| on_wifi(td, &id)).await;
}

#[tokio::test]
async fn usb_wins_and_unplugging_falls_back_to_wifi() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    let rev = td.device(&id).unwrap().rev;

    td.plug_device(SERIAL).await;
    td.until(|td| {
        td.device(&id)
            .is_some_and(|d| d.carrier == Some(Carrier::Usb))
    })
    .await;
    let device = td.device(&id).unwrap();
    assert!(
        matches!(&device.link, LinkState::Connected { port, .. } if port == "mock:020000000001")
    );
    assert!(device.rev > rev);
    assert_eq!(td.device_count(), 1, "one Device on both Carriers");
    td.until(|_| fake.session().is_none()).await;

    td.unplug(SERIAL).await;
    td.until(|td| on_wifi(td, &id)).await;
    assert_eq!(td.device_count(), 1);
    assert_eq!(
        td.device(&id).unwrap().name.as_deref(),
        Some("Mock Device 020000000001")
    );
}

#[tokio::test]
async fn a_layout_push_reaches_the_device_over_wifi() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    let layout: Layout = serde_json::from_value(json!({
        "schema": 1, "name": "Over the air", "orientation": "landscape",
        "pages": [{"id": "main", "title": "Main", "kind": "grid", "rows": 2, "cols": 2, "tiles": [
            {"id": "a", "label": "A", "row": 0, "col": 0, "rowSpan": 1, "colSpan": 1,
             "action": {"type": "none"}}
        ]}]
    }))
    .unwrap();
    let job = td
        .call::<Push>(
            LayoutPath {
                device: id.to_string(),
                orientation: Orientation::Landscape,
            },
            NoQuery,
            LayoutBody::new(&layout),
        )
        .await
        .unwrap();
    let job = td.wait(&job).await;
    assert_eq!(job.state, JobState::Succeeded, "{job:?}");
    assert_eq!(
        fake.deck().layout(Orientation::Landscape),
        Some(layout.to_canonical_json())
    );
}

#[tokio::test]
async fn a_firmware_update_over_wifi_asks_for_usb() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    let (status, error) = failure(td.request::<Update>(
        FirmwareTarget { device: id },
        NoQuery,
        &FirmwareRequest {
            bundle: "bundled".into(),
            channel: None,
        },
    ))
    .await;
    assert_eq!(
        (status, error["code"].as_str()),
        (409, Some("firmware.usb_only"))
    );
    assert_eq!(fake.update_prompt(), None, "nothing reached the Device");
}

#[tokio::test]
async fn forgetting_a_pairing_drops_the_key_and_the_link() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    td.call::<PairingsForget>(PairingPath { device: id.clone() }, NoQuery, ())
        .await
        .unwrap();
    assert!(!td.wifi_keys().has(&id));
    assert!(
        td.call::<PairingsList>((), NoQuery, ())
            .await
            .unwrap()
            .is_empty()
    );
    let device = td.device(&id).unwrap();
    assert_eq!(device.link, LinkState::Offline);
    assert_eq!(device.carrier, None);
    td.until(|_| fake.session().is_none()).await;
    let (status, error) =
        failure(td.request::<PairingsForget>(PairingPath { device: id }, NoQuery, &())).await;
    assert_eq!(
        (status, error["code"].as_str()),
        (404, Some("wifi.not_paired"))
    );
}

/// The Pairings list, read until `done` holds or 5 s pass.
async fn pairings_until(
    td: &TestCompanion,
    done: impl Fn(&[td_companion::devices::WifiPairing]) -> bool,
) -> Vec<td_companion::devices::WifiPairing> {
    let end = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let pairings = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
        if done(&pairings) || std::time::Instant::now() > end {
            return pairings;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn a_version_refusal_stops_wifi_until_the_person_connects_again() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    fake.refuse_sessions_with(Some(net::Refusal::Version));
    fake.drop_connection();
    let pairings = pairings_until(&td, |p| {
        p[0].problem
            .as_deref()
            .is_some_and(|problem| problem.contains("Update its Firmware over USB"))
    })
    .await;
    assert!(
        pairings[0].problem.as_deref().unwrap().contains("over USB"),
        "{pairings:?}"
    );
    assert!(!on_wifi(&td, &id));
    // No retry of its own: the backoff would have tried several times by now.
    let tries = fake.connections();
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(fake.connections(), tries, "a version refusal isn't retried");
    assert!(td.wifi_keys().has(&id), "the key stays");

    // The person updated the Firmware and connects again.
    fake.refuse_sessions_with(None);
    td.call::<Connect>(DevicePath { device: id.clone() }, NoQuery, ())
        .await
        .unwrap();
    td.until(|td| on_wifi(td, &id)).await;
}

#[tokio::test]
async fn a_device_without_link_v2_is_not_linked_over_wifi() {
    let td = TestCompanion::start().await;
    // A protocol-1 Device: its hello offers no link-v2 and carries no id.
    let fake = FakeNetDevice::start(td_link::MockDeck::new());
    let attempt = start(&td, &fake).await;
    let paired = td
        .call::<PairCodeEndpoint>(
            AttemptPath { attempt },
            NoQuery,
            PairCode {
                code: fake.code().unwrap(),
            },
        )
        .await
        .unwrap();
    // The pairing connection may still hold the Device, so the first try can be busy.
    let pairings = pairings_until(&td, |p| {
        p[0].problem
            .as_deref()
            .is_some_and(|problem| problem.contains("over USB"))
    })
    .await;
    assert!(
        pairings[0].problem.as_deref().unwrap().contains("over USB"),
        "{pairings:?}"
    );
    assert!(!on_wifi(&td, &paired.device));
    td.until(|_| fake.session().is_none()).await;
}

#[tokio::test]
async fn an_auth_refusal_keeps_the_key() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    // Someone forgot this app on the Device.
    fake.forget_all();
    fake.drop_connection();
    let pairings = pairings_until(&td, |p| {
        p[0].problem
            .as_deref()
            .is_some_and(|problem| problem.contains("pair again"))
    })
    .await;
    assert!(pairings[0].problem.is_some(), "{pairings:?}");
    assert!(td.wifi_keys().has(&id), "an auth never deletes the key");
}

#[tokio::test]
async fn a_full_device_gets_advice_by_its_advertised_count() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let address = net::Address::from(fake.addr());
    for app in 0..4u8 {
        fake.open_window().unwrap();
        let mut other = pair_when_free(&address, [0x70 + app; 16]);
        other
            .submit(&fake.code().unwrap(), &mut |_| Ok(()))
            .unwrap();
    }
    td.wifi_network().announce(net::mdns::Discovered {
        device_id: fake.device_id(),
        name: "Desk".into(),
        address: fake.addr().into(),
        paired: Some(4),
    });
    let found = td.call::<Discover>((), NoQuery, ()).await.unwrap();
    assert_eq!(found[0].paired, Some(4));
    fake.open_window().unwrap();
    // The fourth pairing's connection can still hold the Device, which answers busy until it closes.
    let end = Instant::now() + Duration::from_secs(5);
    let (status, error) = loop {
        let refused = failure(td.request::<PairStartEndpoint>(
            (),
            NoQuery,
            &PairStart {
                address: fake.addr().to_string(),
            },
        ))
        .await;
        if refused.1["code"] != "wifi.busy" || Instant::now() >= end {
            break refused;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!((status, error["code"].as_str()), (409, Some("wifi.full")));
    assert_eq!(error["details"]["paired"], 4);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("four paired apps"),
        "{error}"
    );
}

#[tokio::test]
async fn a_device_that_still_knows_this_companion_says_so() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    // Forgotten here only: the Device keeps its side.
    td.call::<PairingsForget>(PairingPath { device: id }, NoQuery, ())
        .await
        .unwrap();
    td.until(|_| fake.session().is_none()).await;
    td.wifi_network().announce(net::mdns::Discovered {
        device_id: fake.device_id(),
        name: "Desk".into(),
        address: fake.addr().into(),
        paired: Some(1),
    });
    td.call::<Discover>((), NoQuery, ()).await.unwrap();
    fake.open_window().unwrap();
    let (status, error) = failure(td.request::<PairStartEndpoint>(
        (),
        NoQuery,
        &PairStart {
            address: fake.addr().to_string(),
        },
    ))
    .await;
    assert_eq!(
        (status, error["code"].as_str()),
        (409, Some("wifi.already_paired"))
    );
}

#[tokio::test]
async fn discovery_refreshes_a_moved_devices_address() {
    let td = TestCompanion::start().await;
    let fake = td.wifi_device(SERIAL, false);
    let id = td.pair_wifi(&fake).await;
    // The Device answers at a new address. A closed loopback port stands in for it.
    let moved = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    td.wifi_network().announce(net::mdns::Discovered {
        device_id: fake.device_id(),
        name: "Desk".into(),
        address: moved.into(),
        paired: Some(1),
    });
    td.call::<Discover>((), NoQuery, ()).await.unwrap();
    let pairings = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
    assert_eq!(pairings[0].address, moved.to_string());

    // Back at its own address, the next discovery finds it and Wi-Fi links there.
    td.call::<Disconnect>(DevicePath { device: id.clone() }, NoQuery, ())
        .await
        .unwrap();
    td.wifi_network().announce(net::mdns::Discovered {
        device_id: fake.device_id(),
        name: "Desk".into(),
        address: fake.addr().into(),
        paired: Some(1),
    });
    td.call::<Discover>((), NoQuery, ()).await.unwrap();
    td.call::<Connect>(DevicePath { device: id.clone() }, NoQuery, ())
        .await
        .unwrap();
    td.until(|td| on_wifi(td, &id)).await;
    let pairings = td.call::<PairingsList>((), NoQuery, ()).await.unwrap();
    assert_eq!(pairings[0].address, fake.addr().to_string());
}
