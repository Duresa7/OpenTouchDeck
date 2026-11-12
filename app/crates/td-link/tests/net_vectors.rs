//! net-v2 against `schema/protocol/net-v2-vectors.json`, the values the reference implementation
//! and the firmware also check, including the shared negative vectors.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha512};
use td_link::MockDeck;
use td_link::net::fake::FakeNetDevice;
use td_link::net::wire::{
    self, C2D, D2C, PAIR_ACCEPT, PAIR_CHALLENGE, PAIR_COMMIT, PAIR_OFFER, PAIR_PROOF, PAIR_REQUEST,
    PAIR_START, PAIR_STORED, PairKeys, RecordReceiver, RecordSender, SESSION_CHALLENGE,
    SESSION_OPEN, SESSION_PROOF, SESSION_READY, SessionKeys, SrpServer,
};
use td_link::net::{Address, Credential, NetCarrier, NetError, Refusal, pair};
use td_link::{Session, SessionConfig};
use x25519_dalek::{EphemeralSecret, X25519_BASEPOINT_BYTES, x25519};

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../schema/protocol/net-v2-vectors.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the vectors file")).unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    let text = value.as_str().expect("a hex string");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn array<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().expect("a fixed-size value")
}

fn hex(value: &[u8]) -> String {
    wire::hex(value)
}

#[test]
fn srp_matches_the_pake_vectors() {
    let v = vectors();
    let (inputs, pake) = (&v["inputs"], &v["pake"]);
    let device_id: [u8; 6] = array(&inputs["deviceId"]);
    let companion_id: [u8; 16] = array(&inputs["companionId"]);
    let salt: [u8; 16] = array(&inputs["salt"]);
    let code = inputs["code"].as_str().unwrap();
    let identity = wire::identity(&device_id, &companion_id);
    assert_eq!(identity, pake["identity"].as_str().unwrap().as_bytes());

    let a_pub = wire::srp_client_public(&array(&inputs["a"])).unwrap();
    assert_eq!(hex(&a_pub), pake["A"].as_str().unwrap());
    let server = SrpServer::new(&identity, code, &salt, &array(&inputs["b"])).unwrap();
    assert_eq!(hex(&server.b_pub), pake["B"].as_str().unwrap());
    let u = wire::srp_u(&a_pub, &server.b_pub).unwrap();
    assert_eq!(hex(&wire::min(&u)), pake["u"].as_str().unwrap());

    let k_srp = pake["K_srp"].as_str().unwrap();
    let s = bytes(&pake["S"]);
    assert_eq!(hex(&Sha512::digest(&s)), k_srp, "K_srp = H512(MIN(S))");
    let client =
        wire::srp_client_key(&identity, code, &salt, &array(&inputs["a"]), &server.b_pub).unwrap();
    assert_eq!(hex(&client), k_srp);
    assert_eq!(hex(&server.key(&a_pub).unwrap()), k_srp);
}

#[test]
fn pairing_matches_the_wire_keys_and_proofs() {
    let v = vectors();
    let (inputs, pairing) = (&v["inputs"], &v["pairing"]);
    let wire_items = &pairing["wire"];
    let device_id: [u8; 6] = array(&inputs["deviceId"]);
    let companion_id: [u8; 16] = array(&inputs["companionId"]);
    let name = inputs["companionName"].as_str().unwrap();
    let ltk: [u8; 32] = array(&inputs["ltk"]);

    let request = wire::handshake(
        PAIR_REQUEST,
        &[&companion_id[..], &[name.len() as u8], name.as_bytes()].concat(),
    )
    .unwrap();
    assert_eq!(hex(&request), wire_items["request"].as_str().unwrap());
    let offer = bytes(&wire_items["offer"]);
    let (kind, fields) = wire::parse_handshake(&offer).unwrap();
    assert_eq!(kind, PAIR_OFFER);
    assert_eq!(fields[..6], device_id);
    assert_eq!(fields[6..22], bytes(&inputs["windowId"])[..]);

    let a_pub = wire::srp_client_public(&array(&inputs["a"])).unwrap();
    let start = wire::handshake(
        PAIR_START,
        &[&a_pub[..], &bytes(&inputs["clientNonce"])].concat(),
    )
    .unwrap();
    assert_eq!(hex(&start), wire_items["start"].as_str().unwrap());
    let challenge = bytes(&wire_items["challenge"]);
    assert_eq!(wire::parse_handshake(&challenge).unwrap().0, PAIR_CHALLENGE);

    let k_srp: [u8; 64] = array(&v["pake"]["K_srp"]);
    let keys = PairKeys::derive(&k_srp, &request, &offer, &start, &challenge).unwrap();
    for (name, got) in [
        ("transcript", keys.transcript),
        ("prk", keys.prk),
        ("client", keys.client),
        ("deck", keys.deck),
        ("wrap", keys.wrap),
        ("commit", keys.commit),
        ("clientProof", keys.client_proof),
        ("deckProof", keys.deck_proof),
    ] {
        assert_eq!(hex(&got), pairing[name].as_str().unwrap(), "{name}");
    }
    assert_eq!(
        hex(&keys.wrap_key(&ltk)),
        pairing["wrapped_key"].as_str().unwrap()
    );

    let proof = wire::handshake(PAIR_PROOF, &keys.client_proof).unwrap();
    assert_eq!(hex(&proof), wire_items["proof"].as_str().unwrap());
    let accept = bytes(&wire_items["accept"]);
    let (kind, fields) = wire::parse_handshake(&accept).unwrap();
    assert_eq!(kind, PAIR_ACCEPT);
    assert_eq!(keys.accept(fields).unwrap(), ltk);
    let commit = wire::handshake(PAIR_COMMIT, &keys.commit_proof(&ltk)).unwrap();
    assert_eq!(hex(&commit), wire_items["commit"].as_str().unwrap());
    let stored = wire::handshake(PAIR_STORED, &keys.stored_proof(&ltk)).unwrap();
    assert_eq!(hex(&stored), wire_items["stored"].as_str().unwrap());

    let mut tampered = fields.to_vec();
    tampered[40] ^= 1;
    assert!(
        keys.accept(&tampered).is_err(),
        "a changed wrapped key fails"
    );
    tampered = fields.to_vec();
    tampered[0] ^= 1;
    assert!(
        keys.accept(&tampered).is_err(),
        "a changed device proof fails"
    );
}

#[test]
fn session_matches_the_shares_keys_and_wire() {
    let v = vectors();
    let (inputs, session) = (&v["inputs"], &v["session"]);
    let device_id: [u8; 6] = array(&inputs["deviceId"]);
    let companion_id: [u8; 16] = array(&inputs["companionId"]);
    let client_secret: [u8; 32] = array(&inputs["clientEphemeral"]);
    let client_share = x25519(client_secret, X25519_BASEPOINT_BYTES);
    let device_share = x25519(array(&inputs["deviceEphemeral"]), X25519_BASEPOINT_BYTES);
    assert_eq!(
        hex(&client_share),
        session["client_share"].as_str().unwrap()
    );
    assert_eq!(
        hex(&device_share),
        session["device_share"].as_str().unwrap()
    );
    let z = x25519(client_secret, device_share);
    assert_eq!(hex(&z), session["Z"].as_str().unwrap());

    let keys = SessionKeys::derive(
        &array(&inputs["ltk"]),
        &device_id,
        &companion_id,
        &client_share,
        &device_share,
        &z,
    )
    .unwrap();
    for (name, got) in [
        ("transcript", keys.transcript),
        ("prk", keys.prk),
        ("deck", keys.deck),
        ("client", keys.client),
        ("ready", keys.ready),
        ("c2d", keys.c2d),
        ("d2c", keys.d2c),
        ("deckProof", keys.deck_proof),
        ("clientProof", keys.client_proof),
        ("readyProof", keys.ready_proof),
    ] {
        assert_eq!(hex(&got), session[name].as_str().unwrap(), "{name}");
    }
    let w = &session["wire"];
    let open = wire::handshake(
        SESSION_OPEN,
        &[&device_id[..], &companion_id, &client_share].concat(),
    )
    .unwrap();
    assert_eq!(hex(&open), w["open"].as_str().unwrap());
    let challenge = wire::handshake(
        SESSION_CHALLENGE,
        &[&device_share[..], &keys.deck_proof].concat(),
    )
    .unwrap();
    assert_eq!(hex(&challenge), w["challenge"].as_str().unwrap());
    let proof = wire::handshake(SESSION_PROOF, &keys.client_proof).unwrap();
    assert_eq!(hex(&proof), w["proof"].as_str().unwrap());
    let ready = wire::handshake(SESSION_READY, &keys.ready_proof).unwrap();
    assert_eq!(hex(&ready), w["ready"].as_str().unwrap());
}

fn direction(value: &Value) -> u8 {
    match value.as_str().unwrap() {
        "c2d" => C2D,
        "d2c" => D2C,
        other => panic!("unknown direction {other}"),
    }
}

fn record_key(v: &Value, dir: u8) -> [u8; 32] {
    array(&v["session"][if dir == C2D { "c2d" } else { "d2c" }])
}

#[test]
fn records_seal_and_open_as_the_vectors_say() {
    let v = vectors();
    for record in v["records"].as_array().unwrap() {
        let name = record["name"].as_str().unwrap();
        let dir = direction(&record["direction"]);
        let counter = record["counter"].as_u64().unwrap();
        let plaintext = bytes(&record["plaintext"]);
        let key = record_key(&v, dir);
        let (nonce, aad) =
            wire::record_parameters(dir, counter, plaintext.len() + wire::TAG_LEN).unwrap();
        assert_eq!(
            hex(&nonce),
            record["nonce"].as_str().unwrap(),
            "{name} nonce"
        );
        assert_eq!(hex(&aad), record["aad"].as_str().unwrap(), "{name} aad");
        let sealed = RecordSender::at(&key, dir, counter)
            .seal(&plaintext)
            .unwrap();
        assert_eq!(
            hex(&sealed),
            record["wire"].as_str().unwrap(),
            "{name} wire"
        );
        let opened = RecordReceiver::at(&key, dir, counter)
            .open(&sealed)
            .unwrap();
        assert_eq!(opened, plaintext, "{name} opens");
    }
}

#[test]
fn a_tampered_record_closes_the_receiver() {
    let v = vectors();
    let tampered = &v["tampered_record"];
    assert_eq!(tampered["expected"], "close");
    let dir = direction(&tampered["direction"]);
    let counter = tampered["counter"].as_u64().unwrap();
    let key = record_key(&v, dir);
    let mut receiver = RecordReceiver::at(&key, dir, counter);
    assert!(receiver.open(&bytes(&tampered["wire"])).is_err());
    let genuine = v["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| direction(&r["direction"]) == dir && r["counter"].as_u64() == Some(counter))
        .expect("a genuine record at the same counter");
    assert!(
        receiver.open(&bytes(&genuine["wire"])).is_err(),
        "the closed direction stays closed"
    );
}

#[test]
fn the_error_items_match() {
    let v = vectors();
    assert_eq!(
        hex(&wire::error_item(Refusal::Busy)),
        v["errors"]["busy"].as_str().unwrap()
    );
    assert_eq!(
        hex(&wire::error_item(Refusal::Closed)),
        v["errors"]["closed"].as_str().unwrap()
    );
    assert_eq!(
        hex(&wire::error_item(Refusal::Version)),
        v["errors"]["version"].as_str().unwrap()
    );
}

#[test]
fn a_non_contributory_share_closes_the_handshake() {
    let v = vectors();
    let inputs = &v["inputs"];
    let zero: [u8; 32] = array(&v["non_contributory_share"]["device_share"]);
    assert_eq!(v["non_contributory_share"]["expected"], "close");
    let device_id: [u8; 6] = array(&inputs["deviceId"]);
    let companion_id: [u8; 16] = array(&inputs["companionId"]);
    let client_secret: [u8; 32] = array(&inputs["clientEphemeral"]);
    let z = x25519(client_secret, zero);
    let client_share = x25519(client_secret, X25519_BASEPOINT_BYTES);
    assert!(
        SessionKeys::derive(
            &array(&inputs["ltk"]),
            &device_id,
            &companion_id,
            &client_share,
            &zero,
            &z
        )
        .is_err()
    );

    // A scripted Device that answers session.open with the zero share.
    let challenge =
        wire::handshake(SESSION_CHALLENGE, &[&zero[..], &[0x55u8; 32]].concat()).unwrap();
    client_closes_on(&v, challenge);
}

// ---- The shared negative vectors (`negative` and `companion_names`). ----

/// A scripted Device: it reads the 58-byte `session.open`, answers with `reply`, then
/// returns what else the client sent and how long it took to close.
fn scripted_device(reply: Vec<u8>) -> (Address, thread::JoinHandle<(Vec<u8>, Duration)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = Address::from(listener.local_addr().unwrap());
    let device = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut open = [0u8; 58];
        stream.read_exact(&mut open).unwrap();
        stream.write_all(&reply).unwrap();
        let started = Instant::now();
        let mut rest = Vec::new();
        let _ = stream.read_to_end(&mut rest);
        (rest, started.elapsed())
    });
    (addr, device)
}

fn fixture_credential(v: &Value) -> Credential {
    let inputs = &v["inputs"];
    Credential {
        device_id: array(&inputs["deviceId"]),
        companion_id: array(&inputs["companionId"]),
        ltk: array(&inputs["ltk"]),
    }
}

/// The real client against a Device that answers `session.open` with `challenge`: it closes
/// at once, sends no `session.proof`, and fails with `Auth`.
fn client_closes_on(v: &Value, challenge: Vec<u8>) {
    let (addr, device) = scripted_device(challenge);
    let result = NetCarrier::connect(&addr, &fixture_credential(v), Duration::from_secs(5));
    assert!(
        matches!(result, Err(NetError::Auth(_))),
        "{:?}",
        result.err()
    );
    let (rest, waited) = device.join().unwrap();
    assert!(rest.is_empty(), "no session.proof follows");
    assert!(waited < Duration::from_secs(2), "the client closed at once");
}

/// The pairing keys of the fixture's wire items under `k_srp`.
fn pairing_keys(v: &Value, k_srp: &[u8; 64]) -> PairKeys {
    let w = &v["pairing"]["wire"];
    PairKeys::derive(
        k_srp,
        &bytes(&w["request"]),
        &bytes(&w["offer"]),
        &bytes(&w["start"]),
        &bytes(&w["challenge"]),
    )
    .unwrap()
}

#[test]
fn a_wrong_code_gets_auth() {
    let v = vectors();
    let (inputs, negative) = (&v["inputs"], &v["negative"]);
    let wrong_code = &negative["wrong_code"];
    assert_eq!(
        (
            wrong_code["receiver"].as_str(),
            wrong_code["expected"].as_str()
        ),
        (Some("device"), Some("auth"))
    );
    let identity = wire::identity(&array(&inputs["deviceId"]), &array(&inputs["companionId"]));
    let salt: [u8; 16] = array(&inputs["salt"]);
    let challenge = bytes(&v["pairing"]["wire"]["challenge"]);
    let b_pub: [u8; 384] = wire::parse_handshake(&challenge).unwrap().1[16..400]
        .try_into()
        .unwrap();

    // The client's proof from the wrong code is the vector's.
    let wrong = negative["inputs"]["wrong_code"].as_str().unwrap();
    let k_wrong =
        wire::srp_client_key(&identity, wrong, &salt, &array(&inputs["a"]), &b_pub).unwrap();
    let sent = wire::handshake(PAIR_PROOF, &pairing_keys(&v, &k_wrong).client_proof).unwrap();
    assert_eq!(hex(&sent), wrong_code["proof"].as_str().unwrap());

    // The Device's check, under the right code, fails, and it answers `auth`.
    let server = SrpServer::new(
        &identity,
        inputs["code"].as_str().unwrap(),
        &salt,
        &array(&inputs["b"]),
    )
    .unwrap();
    let a_pub = wire::srp_client_public(&array(&inputs["a"])).unwrap();
    let expected = pairing_keys(&v, &server.key(&a_pub).unwrap()).client_proof;
    let (kind, proof) = wire::parse_handshake(&sent).unwrap();
    assert_eq!(kind, PAIR_PROOF);
    assert!(!wire::same(proof, &expected));
    assert_eq!(
        hex(&wire::error_item(Refusal::Auth)),
        wrong_code["error"].as_str().unwrap()
    );
}

#[test]
fn a_tampered_wrapped_key_closes_without_an_ltk() {
    let v = vectors();
    let keys = pairing_keys(&v, &array(&v["pake"]["K_srp"]));
    let cases = v["negative"]["tampered_wrapped_key"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_eq!(
            (case["receiver"].as_str(), case["expected"].as_str()),
            (Some("companion"), Some("close"))
        );
        let accept = bytes(&case["accept"]);
        let (kind, fields) = wire::parse_handshake(&accept).unwrap();
        assert_eq!(kind, PAIR_ACCEPT);
        assert!(keys.accept(fields).is_err(), "{}", case["note"]);
    }
}

#[test]
fn a_swapped_client_share_fails_the_devices_proof_check() {
    let v = vectors();
    let (inputs, case) = (&v["inputs"], &v["negative"]["swapped_client_share"]);
    assert_eq!(
        (case["receiver"].as_str(), case["expected"].as_str()),
        (Some("device"), Some("close"))
    );
    let open = bytes(&case["open"]);
    let (kind, fields) = wire::parse_handshake(&open).unwrap();
    assert_eq!(kind, SESSION_OPEN);
    let client_share: [u8; 32] = fields[22..54].try_into().unwrap();
    let other = x25519(
        array(&v["negative"]["inputs"]["other_ephemeral"]),
        X25519_BASEPOINT_BYTES,
    );
    assert_eq!(
        client_share, other,
        "an honest share, just not the client's"
    );

    // The Device answers as the vector says...
    let device_secret: [u8; 32] = array(&inputs["deviceEphemeral"]);
    let device_share = x25519(device_secret, X25519_BASEPOINT_BYTES);
    let keys = SessionKeys::derive(
        &array(&inputs["ltk"]),
        &array(&inputs["deviceId"]),
        &array(&inputs["companionId"]),
        &client_share,
        &device_share,
        &x25519(device_secret, client_share),
    )
    .unwrap();
    let challenge = wire::handshake(
        SESSION_CHALLENGE,
        &[&device_share[..], &keys.deck_proof].concat(),
    )
    .unwrap();
    assert_eq!(hex(&challenge), case["challenge"].as_str().unwrap());
    // ...and the original session.proof fails its check.
    let (kind, proof) = wire::parse_handshake(&bytes(&case["proof"]))
        .map(|(k, p)| (k, p.to_vec()))
        .unwrap();
    assert_eq!(kind, SESSION_PROOF);
    assert!(!wire::same(&proof, &keys.client_proof));
}

#[test]
fn a_swapped_device_share_fails_the_companions_proof_check() {
    let v = vectors();
    let (inputs, case) = (&v["inputs"], &v["negative"]["swapped_device_share"]);
    assert_eq!(
        (case["receiver"].as_str(), case["expected"].as_str()),
        (Some("companion"), Some("close"))
    );
    let challenge = bytes(&case["challenge"]);
    let (kind, fields) = wire::parse_handshake(&challenge).unwrap();
    assert_eq!(kind, SESSION_CHALLENGE);
    let device_share: [u8; 32] = fields[..32].try_into().unwrap();
    let client_secret: [u8; 32] = array(&inputs["clientEphemeral"]);
    let keys = SessionKeys::derive(
        &array(&inputs["ltk"]),
        &array(&inputs["deviceId"]),
        &array(&inputs["companionId"]),
        &x25519(client_secret, X25519_BASEPOINT_BYTES),
        &device_share,
        &x25519(client_secret, device_share),
    )
    .unwrap();
    assert!(!wire::same(&fields[32..], &keys.deck_proof));
    client_closes_on(&v, challenge);
}

/// Sends `open` to the fake Device and returns its answer.
fn fake_answers(open: &[u8]) -> Vec<u8> {
    let device = FakeNetDevice::start(MockDeck::v2());
    let mut stream = TcpStream::connect(device.addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(open).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    answer
}

#[test]
fn a_wrong_device_id_or_unknown_companion_id_gets_auth() {
    let v = vectors();
    let negative = &v["negative"];
    assert_eq!(
        FakeNetDevice::start(MockDeck::v2()).device_id(),
        array::<6>(&v["inputs"]["deviceId"]),
        "the fake Device is the fixture's Device"
    );
    for name in ["wrong_device_id", "unknown_companion_id"] {
        let case = &negative[name];
        assert_eq!(
            (case["receiver"].as_str(), case["expected"].as_str()),
            (Some("device"), Some("auth")),
            "{name}"
        );
        assert_eq!(
            hex(&fake_answers(&bytes(&case["open"]))),
            case["error"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn the_counter_limit_opens_the_last_record_and_closes_the_next() {
    let v = vectors();
    let case = &v["negative"]["counter_limit"];
    let dir = direction(&case["direction"]);
    let key = record_key(&v, dir);
    let plaintext = bytes(&case["plaintext"]);
    let last = case["last_counter"].as_u64().unwrap();
    assert_eq!(last, wire::COUNTER_LIMIT - 1);
    assert_eq!(case["last_expected"], "accept");
    let sealed = RecordSender::at(&key, dir, last).seal(&plaintext).unwrap();
    assert_eq!(hex(&sealed), case["last"].as_str().unwrap());
    assert_eq!(
        RecordReceiver::at(&key, dir, last)
            .open(&bytes(&case["last"]))
            .unwrap(),
        plaintext
    );

    assert_eq!(case["limit"].as_u64(), Some(1 << 24));
    assert_eq!(case["limit"].as_u64(), Some(wire::COUNTER_LIMIT));
    assert_eq!(case["at_limit"], "end the session");
    let beyond = case["beyond_counter"].as_u64().unwrap();
    assert_eq!(beyond, wire::COUNTER_LIMIT);
    assert_eq!(case["beyond_expected"], "close");
    assert!(
        RecordReceiver::at(&key, dir, beyond)
            .open(&bytes(&case["beyond"]))
            .is_err()
    );
    assert!(
        RecordSender::at(&key, dir, beyond)
            .seal(&plaintext)
            .is_err()
    );
}

#[test]
fn the_largest_record_opens_and_oversize_prefixes_close() {
    let v = vectors();
    let case = &v["negative"]["max_record"];
    assert_eq!(case["expected"], "accept");
    let dir = direction(&case["direction"]);
    let key = record_key(&v, dir);
    let counter = case["counter"].as_u64().unwrap();
    let item = bytes(&case["wire"]);
    assert_eq!(item.len(), 1_050);
    assert_eq!(
        wire::Phase::Record.body_len([item[0], item[1]]).unwrap(),
        1_048
    );
    let plaintext = bytes(&case["plaintext"]);
    assert_eq!(
        hex(&RecordSender::at(&key, dir, counter)
            .seal(&plaintext)
            .unwrap()),
        case["wire"].as_str().unwrap()
    );
    assert_eq!(
        RecordReceiver::at(&key, dir, counter).open(&item).unwrap(),
        plaintext
    );

    let cases = v["negative"]["oversize_items"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_eq!(case["expected"], "close");
        let phase = match case["phase"].as_str().unwrap() {
            "handshake" => wire::Phase::Handshake,
            "record" => wire::Phase::Record,
            other => panic!("unknown phase {other}"),
        };
        let prefix: [u8; 2] = array(&case["prefix"]);
        assert!(
            phase.body_len(prefix).is_err(),
            "{phase:?} {}",
            case["prefix"]
        );
    }
}

#[test]
fn the_credit_vectors_are_for_ble_only() {
    // The Wi-Fi Carrier has no credit: `write_before_credit` and `credit_overrun` are
    // events on BLE characteristics, which this client doesn't speak.
    let v = vectors();
    for name in ["write_before_credit", "credit_overrun"] {
        let case = &v["negative"][name];
        assert_eq!(case["expected"], "close", "{name}");
        assert!(
            case["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e.get("enable").is_some()),
            "{name}"
        );
    }
}

#[test]
fn every_small_order_or_non_canonical_share_closes() {
    let v = vectors();
    let x = &v["negative"]["x25519"];
    let mut shares: Vec<(String, [u8; 32])> = x["small_order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            assert_eq!(case["expected"], "close");
            (
                case["note"].as_str().unwrap().to_owned(),
                array(&case["share"]),
            )
        })
        .collect();
    assert_eq!(shares.len(), 12, "all twelve small-order encodings");
    assert_eq!(x["bit_255_set"]["expected"], "close");
    let high: [u8; 32] = array(&x["bit_255_set"]["share"]);
    shares.push(("bit 255 set".into(), high));
    for (note, share) in &shares {
        assert!(
            wire::agree(EphemeralSecret::random(), share).is_err(),
            "{note}"
        );
    }
    // The bit-255 share is honest underneath: only the canonical check refuses it.
    let mut masked = high;
    masked[31] &= 0x7f;
    assert!(wire::agree(EphemeralSecret::random(), &masked).is_ok());

    // The real client closes on it before sending a proof.
    let challenge =
        wire::handshake(SESSION_CHALLENGE, &[&high[..], &[0x55u8; 32]].concat()).unwrap();
    client_closes_on(&v, challenge);
}

#[test]
fn companion_names_follow_the_pinned_rule() {
    let v = vectors();
    let names = &v["companion_names"];
    let ranges: Vec<(u32, u32)> = names["format_ranges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|range| {
            let text = range.as_str().unwrap();
            let (lo, hi) = text.split_once('-').unwrap_or((text, text));
            (
                u32::from_str_radix(lo, 16).unwrap(),
                u32::from_str_radix(hi, 16).unwrap(),
            )
        })
        .collect();
    assert_eq!(ranges, wire::FORMAT_RANGES);
    for case in names["accepted"].as_array().unwrap() {
        let request = bytes(&case["request"]);
        assert!(
            wire::parse_handshake(&request).is_ok(),
            "{} is accepted",
            case["label"]
        );
    }
    let rejected = names["rejected"].as_array().unwrap();
    assert!(!rejected.is_empty());
    for case in rejected {
        assert_eq!(case["expected"], "close");
        let request = bytes(&case["request"]);
        assert!(
            wire::parse_handshake(&request).is_err(),
            "{} closes",
            case["label"]
        );
    }
}

/// A scripted Device for any first item: it reads one length-prefixed item, answers with
/// `reply`, then returns what else the client sent and how long it took to close.
fn answers_first_item_with(reply: Vec<u8>) -> (Address, thread::JoinHandle<(Vec<u8>, Duration)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = Address::from(listener.local_addr().unwrap());
    let device = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut prefix = [0u8; 2];
        stream.read_exact(&mut prefix).unwrap();
        let mut item = vec![0u8; usize::from(u16::from_be_bytes(prefix))];
        stream.read_exact(&mut item).unwrap();
        stream.write_all(&reply).unwrap();
        let started = Instant::now();
        let mut rest = Vec::new();
        let _ = stream.read_to_end(&mut rest);
        (rest, started.elapsed())
    });
    (addr, device)
}

#[test]
fn a_version_error_stops_the_companion_with_no_fallback() {
    let v = vectors();
    let case = &v["negative"]["version_error"];
    assert_eq!(
        (case["receiver"].as_str(), case["expected"].as_str()),
        (Some("companion"), Some("stop"))
    );
    assert_eq!(case["outcome"]["fallback"], Value::Null);
    assert_eq!(case["outcome"]["retry"], "person");
    assert_eq!(case["error"], v["errors"]["version"]);
    let error = bytes(&case["error"]);

    // On session.open: the client stops with `version`, sends nothing more and closes.
    let (addr, device) = answers_first_item_with(error.clone());
    let result = NetCarrier::connect(&addr, &fixture_credential(&v), Duration::from_secs(5));
    assert!(
        matches!(result, Err(NetError::Refused(Refusal::Version))),
        "{:?}",
        result.err()
    );
    let (rest, waited) = device.join().unwrap();
    assert!(rest.is_empty(), "no net-v1 item, no plaintext and no retry");
    assert!(waited < Duration::from_secs(2), "the client closed at once");

    // On pair.request too.
    let (addr, device) = answers_first_item_with(error);
    let result = pair(&addr, &[0x10; 16], "Vector app", Duration::from_secs(5));
    assert!(
        matches!(result, Err(NetError::Refused(Refusal::Version))),
        "{:?}",
        result.err()
    );
    let (rest, _) = device.join().unwrap();
    assert!(rest.is_empty());

    // The same error item with item version 1 isn't an answer: the client closes.
    let v1 = &case["v1_item"];
    assert_eq!(v1["expected"], "close");
    let (addr, device) = answers_first_item_with(bytes(&v1["wire"]));
    let result = NetCarrier::connect(&addr, &fixture_credential(&v), Duration::from_secs(5));
    assert!(
        matches!(result, Err(NetError::Version(1))),
        "{:?}",
        result.err()
    );
    let (rest, _) = device.join().unwrap();
    assert!(rest.is_empty());
}

/// The `management` steps a network session can run against the fake Device: the USB-only
/// refusal over Wi-Fi. `management_over_usb` runs the USB steps; the BLE step needs a BLE
/// carrier and is skipped.
#[test]
fn management_over_wifi_is_usb_only() {
    let v = vectors();
    let steps = v["management"]["steps"].as_array().unwrap();
    let device = FakeNetDevice::start(MockDeck::v2());
    device.open_window().unwrap();
    let address = Address::from(device.addr());
    let mut pairing = pair(&address, &[0x10; 16], "Vector app", Duration::from_secs(5)).unwrap();
    let paired = pairing
        .submit(&device.code().unwrap(), &mut |_| Ok(()))
        .unwrap();
    let carrier =
        NetCarrier::connect(&address, &paired.credential, Duration::from_secs(5)).unwrap();
    let (session, _incoming) = Session::over(
        Box::new(carrier),
        SessionConfig {
            ping_every: None,
            ..SessionConfig::default()
        },
    )
    .unwrap();
    let mut ran = 0;
    for step in steps {
        if step["carrier"] != "wifi" {
            continue;
        }
        let reply = session
            .call(&step["request"], Duration::from_secs(5))
            .unwrap();
        let expected = &step["response"];
        assert_eq!(reply.m, expected["m"].as_str().unwrap());
        assert_eq!(reply.ok, expected["ok"].as_bool().unwrap());
        for field in ["reason", "error"] {
            assert_eq!(reply.fields.get(field), expected.get(field), "{field}");
        }
        ran += 1;
    }
    assert_eq!(ran, 1, "one step runs over Wi-Fi");
    // net.list is USB-only on every network carrier, so over Wi-Fi it is refused the same way.
    let reply = session
        .call(
            &serde_json::json!({"m": "net.list"}),
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(!reply.ok);
    assert_eq!(reply.fields["reason"], "usb-only");
}

/// Every USB step of the shared `management` vector against the mock Device holding the
/// vector's four apps: each answer must match exactly.
#[test]
fn management_over_usb() {
    let v = vectors();
    let management = &v["management"];
    let deck = MockDeck::v2();
    deck.set_wifi_capability(true);
    for app in management["apps"].as_array().unwrap() {
        deck.add_net_pairing(array(&app["app_id"]), app["app_name"].as_str().unwrap());
    }
    let failures = management["base"]["pair_failures"].as_u64().unwrap() as u32;
    deck.set_net_failures(failures, Duration::ZERO);
    let (session, _) = Session::open(
        Box::new(deck.connect()),
        SessionConfig {
            ping_every: None,
            ..SessionConfig::default()
        },
    )
    .unwrap();
    session.hello(Duration::from_secs(1)).unwrap();
    let mut ran = 0;
    for step in management["steps"].as_array().unwrap() {
        if step["carrier"] != "usb" {
            continue;
        }
        let reply = session
            .call(&step["request"], Duration::from_secs(5))
            .unwrap();
        let mut got = reply.fields.clone();
        got.insert("m".into(), Value::from(reply.m.clone()));
        got.insert("ok".into(), Value::from(reply.ok));
        assert_eq!(
            Value::Object(got),
            step["response"],
            "{}",
            step["note"].as_str().unwrap()
        );
        ran += 1;
    }
    assert_eq!(ran, 8, "eight steps run over USB");
    assert!(deck.net_pairings().is_empty());
}
