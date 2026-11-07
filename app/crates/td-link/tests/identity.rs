use serde_json::json;
use std::time::Duration;
use td_link::{MockDeck, Session, SessionConfig};
const TIMEOUT: Duration = Duration::from_secs(1);
fn open(device: &MockDeck) -> Session {
    Session::open(
        Box::new(device.connect()),
        SessionConfig {
            ping_every: None,
            ..SessionConfig::default()
        },
    )
    .unwrap()
    .0
}
#[test]
fn legacy_firmware_remains_usable_without_v2_requests() {
    let device = MockDeck::new();
    let session = open(&device);
    let identity = session.identity(TIMEOUT).unwrap();
    assert_eq!(identity.protocol, 1);
    assert!(identity.name.is_none() && identity.capabilities.is_none() && identity.id.is_none());
    assert!(session.set_name("Desk", TIMEOUT).is_err());
    assert!(session.hello(TIMEOUT).unwrap().is_deck());
    assert!(device.received().iter().all(|v| v["m"] == "hello"));
}
#[test]
fn negotiated_name_round_trips_and_survives_reconnect() {
    let device = MockDeck::v2();
    {
        let session = open(&device);
        let identity = session.identity(TIMEOUT).unwrap();
        assert!(identity.matches_usb_serial(device.usb_serial()));
        assert!(!identity.matches_usb_serial("02:00:00:00:00:02"));
        let mut caps = identity.capabilities.unwrap();
        caps.tile_kinds.sort();
        assert_eq!(caps.tile_kinds, ["clock", "graph", "photo-frame", "static"]);
        assert_eq!(caps.photo_bytes, 131_072);
        assert_eq!(
            session.set_name("  Studio \u{1f39b}  ", TIMEOUT).unwrap(),
            "  Studio \u{1f39b}  "
        );
        assert_eq!(session.name(TIMEOUT).unwrap(), "  Studio \u{1f39b}  ");
    }
    assert_eq!(open(&device).name(TIMEOUT).unwrap(), "  Studio \u{1f39b}  ");
}
#[test]
fn invalid_names_preserve_the_committed_name() {
    let device = MockDeck::v2();
    let session = open(&device);
    session.identity(TIMEOUT).unwrap();
    for name in [
        "".to_owned(),
        " \u{2003}".into(),
        "bad\nname".into(),
        "bad\0name".into(),
        "x".repeat(49),
        "\u{1f39b}".repeat(13),
    ] {
        assert!(session.set_name(&name, TIMEOUT).is_err());
        let reply = session
            .call(&json!({"m":"v2.name.set","name":name}), TIMEOUT)
            .unwrap();
        assert!(!reply.ok);
        assert_eq!(reply.fields["reason"], "args");
    }
    assert_eq!(session.name(TIMEOUT).unwrap(), "Open TouchDeck");
    assert_eq!(
        session.set_name(&"x".repeat(48), TIMEOUT).unwrap(),
        "x".repeat(48)
    );
}
#[test]
fn selection_and_unsupported_requests_have_stable_errors() {
    let device = MockDeck::v2();
    let session = open(&device);
    let request = json!({"m":"v2.unknown.get"});
    assert_eq!(
        session.call(&request, TIMEOUT).unwrap().fields["reason"],
        "not-negotiated"
    );
    session.identity(TIMEOUT).unwrap();
    assert_eq!(
        session.call(&request, TIMEOUT).unwrap().fields["reason"],
        "unsupported"
    );
    session.hello(TIMEOUT).unwrap();
    assert_eq!(
        session.call(&request, TIMEOUT).unwrap().fields["reason"],
        "not-negotiated"
    );
}

#[test]
fn discovery_matches_normalized_usb_serial_and_refuses_mismatch() {
    use td_link::discover::probe_identity_transport;
    let device = MockDeck::v2();
    for serial in [device.usb_serial(), "02-00-00-00-00-01", "020000000001"] {
        let identity = probe_identity_transport(Box::new(device.connect()), Some(serial)).unwrap();
        assert_eq!(identity.id.as_deref(), Some("020000000001"));
    }
    for serial in [None, Some("020000000002")] {
        assert!(probe_identity_transport(Box::new(device.connect()), serial).is_err());
    }
    let mut identity = open(&device).identity(TIMEOUT).unwrap();
    identity.id = Some("aabbccddeeff".into());
    assert!(identity.matches_usb_serial("AA:BB:CC:DD:EE:FF"));
    let legacy = MockDeck::new();
    assert_eq!(
        probe_identity_transport(Box::new(legacy.connect()), None)
            .unwrap()
            .protocol,
        1
    );
}

#[test]
fn gif_caps_are_typed_opt_in_and_match_golden_hello() {
    let vectors: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../schema/protocol/golden-hello.json"
    ))
    .unwrap();
    assert_eq!(vectors.len(), 4);
    let switches = [(false, false), (true, false), (false, true), (true, true)];
    for ((gif, media), vector) in switches.into_iter().zip(&vectors) {
        let device = MockDeck::v2();
        device.set_gif_capabilities(gif);
        device.set_media_capabilities(media);
        let mut caps = open(&device)
            .identity(TIMEOUT)
            .unwrap()
            .capabilities
            .unwrap();
        assert_eq!(caps.gif_enabled(), gif);
        assert_eq!(caps.live_images.unwrap_or(0), if media { 4 } else { 0 });
        caps.tile_kinds.reverse();
        caps.formats.reverse();
        caps.layout_schemas.reverse();
        assert_eq!(caps.gif_enabled(), gif);
        let mut expected: td_link::Capabilities =
            serde_json::from_value(vector["caps"].clone()).unwrap();
        caps.free_bytes = expected.free_bytes;
        caps.tile_kinds.sort();
        caps.formats.sort();
        caps.layout_schemas.sort();
        expected.tile_kinds.sort();
        expected.formats.sort();
        expected.layout_schemas.sort();
        assert_eq!(caps, expected, "gif {gif}, media {media}");
    }
    let mut caps: td_link::Capabilities =
        serde_json::from_value(vectors[1]["caps"].clone()).unwrap();
    assert_eq!(caps.anim.as_ref().unwrap().image, [128, 128]);
    caps.anim = None;
    assert!(!caps.gif_enabled());
}

#[test]
fn longest_hello_fits_one_message() {
    let mut vectors: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../schema/protocol/golden-hello.json"
    ))
    .unwrap();
    for hello in &mut vectors {
        hello["name"] = json!("\"".repeat(48));
        hello["version"] = json!("\"".repeat(32));
        hello["running"] = json!("unknown");
        hello["caps"]["free_bytes"] = json!(u32::MAX);
        assert!(serde_json::to_vec(hello).unwrap().len() <= 1024);
    }
}
fn name_vectors() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../schema/protocol/name-vectors-v2.json"
    ))
    .unwrap()
}
fn names(vectors: &serde_json::Value, key: &str) -> Vec<(String, String)> {
    vectors[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["label"].as_str().unwrap().to_owned(),
                v["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}
#[test]
fn shared_name_vectors_decide_valid_name() {
    let vectors = name_vectors();
    for (label, name) in names(&vectors, "accepted") {
        assert!(td_link::identity::valid_name(&name), "{label}");
    }
    for (label, name) in names(&vectors, "rejected") {
        assert!(!td_link::identity::valid_name(&name), "{label}");
    }
}
#[test]
fn only_the_pinned_table_and_the_fillers_hide_a_name() {
    let vectors = name_vectors();
    let mut hidden = std::collections::BTreeSet::new();
    for range in vectors["format_ranges"].as_array().unwrap() {
        let range = range.as_str().unwrap();
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        let (first, last) = (
            u32::from_str_radix(first, 16).unwrap(),
            u32::from_str_radix(last, 16).unwrap(),
        );
        hidden.extend(first..=last);
    }
    for filler in vectors["hangul_fillers"].as_array().unwrap() {
        hidden.insert(u32::from_str_radix(filler.as_str().unwrap(), 16).unwrap());
    }
    for code in 0x20..=0x10ffffu32 {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        if code == 0x7f {
            continue;
        }
        assert_eq!(
            td_link::identity::valid_name(&format!("a{c}b")),
            !hidden.contains(&code),
            "U+{code:04X}"
        );
    }
}
#[test]
fn a_hidden_character_name_is_refused_with_args_and_keeps_the_old_name() {
    let device = MockDeck::v2();
    let session = open(&device);
    session.identity(TIMEOUT).unwrap();
    for name in ["Desk\u{202e}PC", "\u{3164}", "a\u{200d}b", "\u{feff}Desk"] {
        assert!(session.set_name(name, TIMEOUT).is_err(), "{name:?}");
        let reply = session
            .call(&json!({"m":"v2.name.set","name":name}), TIMEOUT)
            .unwrap();
        assert!(!reply.ok);
        assert_eq!(reply.fields["reason"], "args");
    }
    assert_eq!(session.name(TIMEOUT).unwrap(), "Open TouchDeck");
}
