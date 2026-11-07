//! net-v2 end to end on loopback: the Companion's client against the fake Device.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use td_core::{Layout, Orientation};
use td_link::net::fake::FakeNetDevice;
use td_link::net::{Address, Credential, NetCarrier, NetError, Paired, Pairing, Refusal, pair};
use td_link::update::{Approval, begin};
use td_link::{Incoming, IncomingReceiver, MockDeck, Session, SessionConfig, xfer};

const TIMEOUT: Duration = Duration::from_secs(5);
const COMPANION: [u8; 16] = [0x10; 16];
const GRAPH: &[u8] = include_bytes!("../../../../schema/examples/graph.json");

fn fake() -> FakeNetDevice {
    FakeNetDevice::start(MockDeck::v2())
}

fn addr(device: &FakeNetDevice) -> Address {
    Address::from(device.addr())
}

/// Pairs once the fake Device has let go of the previous connection: right after a pairing it can still
/// answer `busy` for a moment, more often on a loaded CI runner.
fn pair_when_free(device: &FakeNetDevice, companion: [u8; 16]) -> Result<Pairing, NetError> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match pair(&addr(device), &companion, "Test workstation", TIMEOUT) {
            Err(NetError::Refused(Refusal::Busy)) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

fn pair_with(device: &FakeNetDevice, companion: [u8; 16]) -> Paired {
    device.open_window().unwrap();
    let mut pairing = pair_when_free(device, companion).unwrap();
    let code = device.code().expect("the Device shows a code");
    pairing.submit(&code, &mut |_| Ok(())).unwrap()
}

fn wrong(code: &str) -> String {
    let n: u32 = code.parse().unwrap();
    format!("{:06}", (n + 1) % 1_000_000)
}

fn quiet() -> SessionConfig {
    SessionConfig {
        ping_every: None,
        ..SessionConfig::default()
    }
}

/// Connects once the fake Device has let go of the previous connection, which it can still hold for a
/// moment after a pairing or a closed session.
fn try_connect_when_free(
    device: &FakeNetDevice,
    credential: &Credential,
) -> Result<NetCarrier, NetError> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match NetCarrier::connect(&addr(device), credential, TIMEOUT) {
            Err(NetError::Refused(Refusal::Busy)) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

fn connect_when_free(device: &FakeNetDevice, credential: &Credential) -> NetCarrier {
    try_connect_when_free(device, credential).unwrap()
}

fn session(device: &FakeNetDevice, credential: &Credential) -> (Session, IncomingReceiver) {
    let carrier = connect_when_free(device, credential);
    Session::over(Box::new(carrier), quiet()).unwrap()
}

fn closed(incoming: &IncomingReceiver) -> bool {
    let end = Instant::now() + TIMEOUT;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Ok(Incoming::Closed(_)) = incoming.recv_timeout(left) {
            return true;
        }
    }
    false
}

#[test]
fn the_right_code_pairs_and_a_session_reaches_the_device() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    assert!(paired.confirmed);
    assert_eq!(paired.credential.device_id, device.device_id());
    assert_eq!(device.paired(), 1);
    assert_eq!(device.paired_names(), ["Test workstation"]);
    assert!(!device.pairing(), "a Pairing closes the window");

    let (session, _incoming) = session(&device, &paired.credential);
    let identity = session.identity(TIMEOUT).unwrap();
    assert_eq!(identity.protocol, 2);
    assert_eq!(identity.id.as_deref(), Some("020000000001"));
    assert_eq!(device.session().as_deref(), Some("Test workstation"));

    let layout = Layout::parse(GRAPH).unwrap();
    xfer::put_layout(&session, Orientation::Landscape, &layout).unwrap();
    let stored = xfer::get_layout(&session, Orientation::Landscape).unwrap();
    assert_eq!(stored.data, layout.to_canonical_json());
    assert_eq!(
        device.deck().layout(Orientation::Landscape),
        Some(layout.to_canonical_json())
    );
}

#[test]
fn a_wrong_code_leaves_the_window_open_for_another() {
    let device = fake();
    device.open_window().unwrap();
    let mut pairing = pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).unwrap();
    let code = device.code().unwrap();
    assert!(matches!(
        pairing.submit(&wrong(&code), &mut |_| Ok(())),
        Err(NetError::Refused(Refusal::Auth))
    ));
    assert_eq!(pairing.attempts_left(), 2);
    assert!(pairing.is_open());
    assert_eq!(device.pair_failures(), 1);
    let paired = pairing.submit(&code, &mut |_| Ok(())).unwrap();
    assert!(paired.confirmed);
    assert_eq!(device.pair_failures(), 0, "a Pairing clears the counter");
}

#[test]
fn three_wrong_codes_lock_pairing_out() {
    let device = fake();
    device.open_window().unwrap();
    let mut pairing = pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).unwrap();
    let code = device.code().unwrap();
    for left in [2, 1, 0] {
        assert!(matches!(
            pairing.submit(&wrong(&code), &mut |_| Ok(())),
            Err(NetError::Refused(Refusal::Auth))
        ));
        assert_eq!(pairing.attempts_left(), left);
    }
    assert!(!pairing.is_open());
    assert!(device.locked_for() > Duration::from_secs(25));
    assert!(
        device.open_window().is_err(),
        "the Device refuses to open a window"
    );
    assert!(matches!(
        pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).err(),
        Some(NetError::Refused(Refusal::Closed))
    ));
    assert_eq!(device.paired(), 0);
}

#[test]
fn pairing_needs_a_window_opened_on_the_device() {
    let device = fake();
    assert!(matches!(
        pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).err(),
        Some(NetError::Refused(Refusal::Closed))
    ));
}

#[test]
fn cancel_on_the_device_ends_the_pairing() {
    let device = fake();
    device.open_window().unwrap();
    let mut pairing = pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).unwrap();
    let code = device.code().unwrap();
    device.cancel_window();
    assert!(matches!(
        pairing.submit(&code, &mut |_| Ok(())),
        Err(NetError::Refused(Refusal::Cancelled) | NetError::Closed)
    ));
    assert_eq!(device.paired(), 0);
}

#[test]
fn a_dropped_window_counts_as_a_failed_code() {
    let device = fake();
    device.open_window().unwrap();
    let pairing = pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).unwrap();
    assert!(device.code().is_some());
    drop(pairing);
    let end = Instant::now() + TIMEOUT;
    while device.pair_failures() == 0 && Instant::now() < end {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        device.pair_failures(),
        1,
        "binding a window and dropping it isn't free"
    );
    assert_eq!(device.code(), None, "the window closed");

    device.open_window().unwrap();
    let pairing = pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).unwrap();
    device.cancel_window();
    drop(pairing);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        device.pair_failures(),
        1,
        "Cancel on the Device doesn't count"
    );
}

#[test]
fn a_failed_key_save_stops_before_commit() {
    let device = fake();
    device.open_window().unwrap();
    let mut pairing = pair(&addr(&device), &COMPANION, "Test workstation", TIMEOUT).unwrap();
    let code = device.code().unwrap();
    let error = pairing
        .submit(&code, &mut |_| Err("no keychain".into()))
        .unwrap_err();
    assert!(matches!(error, NetError::Store(_)), "{error}");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        device.paired(),
        0,
        "the Device discards the uncommitted key"
    );
}

#[test]
fn a_lost_pair_stored_leaves_the_pairing_pending_until_a_session_works() {
    let device = fake();
    device.lose_stored();
    let paired = pair_with(&device, COMPANION);
    assert!(!paired.confirmed);
    assert_eq!(device.paired(), 1);
    let (session, _incoming) = session(&device, &paired.credential);
    assert_eq!(session.identity(TIMEOUT).unwrap().protocol, 2);
}

#[test]
fn an_unpaired_companion_or_another_device_is_refused() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let stranger = Credential {
        companion_id: [0x20; 16],
        ..paired.credential.clone()
    };
    assert!(matches!(
        try_connect_when_free(&device, &stranger).err(),
        Some(NetError::Refused(Refusal::Auth))
    ));
    let elsewhere = Credential {
        device_id: [2, 0, 0, 0, 0, 9],
        ..paired.credential.clone()
    };
    assert!(matches!(
        try_connect_when_free(&device, &elsewhere).err(),
        Some(NetError::Refused(Refusal::Auth))
    ));
    let forged = Credential {
        ltk: [0x55; 32],
        ..paired.credential.clone()
    };
    assert!(NetCarrier::connect(&addr(&device), &forged, TIMEOUT).is_err());
}

#[test]
fn a_device_online_over_usb_is_busy() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (usb, _usb_incoming) = Session::open(Box::new(device.deck().connect()), quiet()).unwrap();
    assert_eq!(usb.identity(TIMEOUT).unwrap().protocol, 2);
    assert!(device.deck().usb_online());
    assert!(matches!(
        NetCarrier::connect(&addr(&device), &paired.credential, TIMEOUT).err(),
        Some(NetError::Refused(Refusal::Busy))
    ));
    usb.close();
    device.deck().hang_up();
    let (session, _incoming) = session(&device, &paired.credential);
    assert_eq!(session.identity(TIMEOUT).unwrap().protocol, 2);
}

#[test]
fn a_second_connection_is_busy_while_a_session_runs() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (_session, _incoming) = session(&device, &paired.credential);
    assert!(matches!(
        NetCarrier::connect(&addr(&device), &paired.credential, TIMEOUT).err(),
        Some(NetError::Refused(Refusal::Busy))
    ));
}

#[test]
fn a_dropped_connection_closes_the_session_and_a_new_one_reconnects() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (session, incoming) = session(&device, &paired.credential);
    assert_eq!(session.identity(TIMEOUT).unwrap().protocol, 2);
    device.drop_connection();
    assert!(closed(&incoming));
    let deadline = Instant::now() + TIMEOUT;
    let (again, _incoming) = loop {
        match NetCarrier::connect(&addr(&device), &paired.credential, TIMEOUT) {
            Ok(carrier) => break Session::over(Box::new(carrier), quiet()).unwrap(),
            Err(NetError::Refused(Refusal::Busy)) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("reconnect failed: {error}"),
        }
    };
    assert_eq!(again.identity(TIMEOUT).unwrap().protocol, 2);
    assert_eq!(device.sessions(), 2);
}

#[test]
fn usb_only_requests_are_refused_over_the_network() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (session, _incoming) = session(&device, &paired.credential);
    let reply = session.call(&json!({"m":"net.forget"}), TIMEOUT).unwrap();
    assert!(!reply.ok);
    assert_eq!(reply.fields["reason"], "usb-only");
    assert_eq!(device.paired(), 1);
    assert!(
        session.send_line("reboot").is_err(),
        "a network carrier has no console"
    );
}

fn image() -> (usize, String) {
    (4096, "ab".repeat(32))
}

#[test]
fn a_firmware_update_opens_only_after_install_on_the_device() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (session, _incoming) = session(&device, &paired.credential);
    let (bytes, sha) = image();
    let approval = Approval {
        repeat: Duration::from_millis(50),
        limit: TIMEOUT,
    };
    assert_eq!(
        device.update_prompt(),
        None,
        "no prompt before update.begin"
    );
    let mut told = 0;
    let opened = thread::scope(|scope| {
        let waiting = scope.spawn(|| begin(&session, bytes, &sha, approval, &mut || told += 1));
        let end = Instant::now() + TIMEOUT;
        while device.update_prompt().is_none() && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(device.update_prompt(), Some((bytes as u64, sha.clone())));
        thread::sleep(Duration::from_millis(120));
        assert!(device.tap_install());
        waiting.join().unwrap()
    })
    .unwrap();
    assert_eq!(told, 1, "the person is told once to look at the Device");
    assert!(opened["xfer"].as_u64().is_some_and(|x| x > 0), "{opened}");
}

#[test]
fn cancel_on_the_device_declines_the_update() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (session, _incoming) = session(&device, &paired.credential);
    let (bytes, sha) = image();
    let approval = Approval {
        repeat: Duration::from_millis(50),
        limit: TIMEOUT,
    };
    let result = thread::scope(|scope| {
        let waiting = scope.spawn(|| begin(&session, bytes, &sha, approval, &mut || {}));
        let end = Instant::now() + TIMEOUT;
        while device.update_prompt().is_none() && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
        device.tap_cancel();
        waiting.join().unwrap()
    });
    assert_eq!(result.unwrap_err(), "the update was declined on the Device");
}

#[test]
fn an_unapproved_prompt_starts_a_cooldown_that_doubles() {
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (session, _incoming) = session(&device, &paired.credential);
    let (bytes, sha) = image();
    let approval = Approval {
        repeat: Duration::from_millis(50),
        limit: TIMEOUT,
    };
    let cancelled = |session: &Session| {
        thread::scope(|scope| {
            let waiting = scope.spawn(|| begin(session, bytes, &sha, approval, &mut || {}));
            let end = Instant::now() + TIMEOUT;
            while device.update_prompt().is_none() && Instant::now() < end {
                thread::sleep(Duration::from_millis(10));
            }
            device.tap_cancel();
            waiting.join().unwrap()
        })
    };
    assert!(cancelled(&session).is_err());
    let left = device.update_cooldown().expect("a cooldown");
    assert!(
        left > Duration::from_secs(55) && left <= Duration::from_secs(60),
        "{left:?}"
    );

    // During the cooldown update.begin is declined at once and shows nothing.
    let mut told = 0;
    let result = begin(&session, bytes, &sha, approval, &mut || told += 1);
    assert_eq!(result.unwrap_err(), "the update was declined on the Device");
    assert_eq!((told, device.update_prompt()), (0, None));

    // After it, a new prompt shows; cancelling that one doubles the cooldown.
    device.end_update_cooldown();
    assert!(cancelled(&session).is_err());
    let left = device.update_cooldown().expect("a second cooldown");
    assert!(
        left > Duration::from_secs(115) && left <= Duration::from_secs(120),
        "{left:?}"
    );
}

#[test]
fn a_full_device_refuses_a_fifth_pairing() {
    let device = fake();
    for n in 0..4u8 {
        pair_with(&device, [n; 16]);
    }
    device.open_window().unwrap();
    assert!(matches!(
        pair_when_free(&device, [9; 16]).err(),
        Some(NetError::Refused(Refusal::Full))
    ));
}

#[test]
fn a_session_near_the_record_limit_ends_between_requests_and_a_new_one_starts_at_zero() {
    use td_link::LinkError;
    use td_link::net::wire::{COUNTER_LIMIT, RECORD_RESERVE};
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    // Both sides start 1025 records short of the limit.
    let start = COUNTER_LIMIT - RECORD_RESERVE - 1;
    device.skip_next_session(start);
    let mut carrier = connect_when_free(&device, &paired.credential);
    assert_eq!(carrier.counters(), (0, 0));
    carrier.skip_records(start);
    let (session, incoming) = Session::over(Box::new(carrier), quiet()).unwrap();
    let ping = json!({"m": "ping"});
    // 1025 left, then 1024: both requests go out and are answered.
    session.call(&ping, TIMEOUT).unwrap();
    session.call(&ping, TIMEOUT).unwrap();
    // 1023 left: the session ends before the request, so the request never went out.
    let error = session.call(&ping, TIMEOUT).unwrap_err();
    assert!(matches!(error, LinkError::Io(_)), "{error:?}");
    assert!(closed(&incoming), "the session ended");
    let end = Instant::now() + TIMEOUT;
    while device.session().is_some() && Instant::now() < end {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(device.session(), None);

    // A reconnect runs the session exchange again: fresh keys with counters at zero.
    let carrier = connect_when_free(&device, &paired.credential);
    assert_eq!(carrier.counters(), (0, 0));
    let (session, _incoming) = Session::over(Box::new(carrier), quiet()).unwrap();
    session.call(&ping, TIMEOUT).unwrap();
    assert_eq!(device.sessions(), 2);
}

#[test]
fn a_request_cut_off_by_a_closed_session_has_an_unknown_outcome() {
    use td_link::LinkError;
    let device = fake();
    let paired = pair_with(&device, COMPANION);
    let (session, _incoming) = session(&device, &paired.credential);
    device.deck().delay_replies(Duration::from_millis(500));
    let dropper = {
        let device = &device;
        thread::scope(|scope| {
            let call = scope.spawn(|| session.call(&json!({"m": "ping"}), TIMEOUT));
            thread::sleep(Duration::from_millis(100));
            device.drop_connection();
            call.join().unwrap()
        })
    };
    assert!(
        matches!(dropper, Err(LinkError::Unanswered(_))),
        "{dropper:?}"
    );
}
