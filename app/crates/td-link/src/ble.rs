//! The radio build's Bluetooth keyboard, as `schema/protocol/link-v2.md` § "Bluetooth keyboard (radio build)" defines
//! it.
//!
//! A Device with `caps.ble_hid` is a Bluetooth LE keyboard with media keys for other hosts. It stays off until the
//! person turns it on. `ble.state` reads it, `ble.set` turns it on or off, `ble.pair` opens or closes the pairing
//! window, and `ble.forget` forgets one bonded host or all of them. All four are USB-only: over any other Carrier the
//! Device refuses them with `usb-only` and changes nothing.
//!
//! A host pairs inside the window by typing the six-digit passkey the Device shows. The code stays on the Device's
//! screen: the state says only whether one shows (`passkey`).

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use td_core::Reply;

use crate::session::{LinkError, Session};

/// The most bonds `caps.ble_hid` may advertise.
pub const BONDS_MAX: u32 = 8;
/// How long `ble.pair` keeps the window open.
pub const PAIRING_MS: u32 = 120_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// `ble.set` starts or stops the controller and the host before it answers.
const SET_TIMEOUT: Duration = Duration::from_secs(5);

/// `aa:bb:cc:dd:ee:ff`: six bytes in lower-case hex, most significant first, as the Device lists them.
pub fn valid_address(address: &str) -> bool {
    address.len() == 17
        && address.split(':').count() == 6
        && address.split(':').all(|byte| {
            byte.len() == 2
                && byte
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

/// An address in either case, as the Firmware takes it, in the lower case the Device lists. `None` when it isn't six
/// hex bytes separated by colons.
pub fn normalize_address(address: &str) -> Option<String> {
    let lower = address.to_ascii_lowercase();
    valid_address(&lower).then_some(lower)
}

/// Whether the controller and the host run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BleStack {
    Off,
    /// The controller and the host are coming up.
    Starting,
    On,
    /// They did not start. The setting stays on.
    Failed,
}

/// What the Device advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BleAdvertising {
    Off,
    /// The pairing window is open.
    Pairing,
    /// For bonded hosts, every 20-30 ms.
    Fast,
    /// For bonded hosts, every 1-2.5 s.
    Slow,
}

/// One bonded host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleBond {
    /// The host's identity address, `aa:bb:cc:dd:ee:ff`.
    pub address: String,
    /// A random static identity address rather than a public one.
    pub random: bool,
    /// A link to this host is up, encrypted with authenticated keys, and bonded.
    pub connected: bool,
}

/// The keyboard's state after a request took effect. Never the passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleState {
    /// The stored setting.
    pub enabled: bool,
    pub state: BleStack,
    /// Whether the pairing window is open.
    pub pairing: bool,
    /// Milliseconds left in the window, 0 when closed.
    pub pairing_ms: u32,
    /// Whether a pairing passkey is on the Device's screen.
    pub passkey: bool,
    pub advertising: BleAdvertising,
    /// The bonded hosts the Device knows of. The list may have hosts in any state: after a host reset it says
    /// `starting` and still lists them.
    pub bonds: Vec<BleBond>,
    /// The same number as `caps.ble_hid`.
    pub bonds_max: u32,
    /// The Battery Level the Device reports to hosts, fixed at 100.
    pub battery: u8,
}

impl BleState {
    fn valid(&self) -> bool {
        (1..=BONDS_MAX).contains(&self.bonds_max)
            && self.bonds.len() <= self.bonds_max as usize
            && self.pairing_ms <= PAIRING_MS
            && (self.pairing || self.pairing_ms == 0)
            && self.battery <= 100
            && self.bonds.iter().all(|bond| valid_address(&bond.address))
    }
}

/// What `ble.forget` answers: how many bonds it deleted, and the state after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleForgotten {
    pub forgotten: u32,
    pub state: BleState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BleError {
    Link(LinkError),
    /// The Device refused with this stable reason: `usb-only`, `off`, `starting`, `full`, `storage`, `radio`, `args`,
    /// `unsupported`, or none for a Device that doesn't know `ble.*`. Every refusal but `args`, `unsupported` and
    /// `usb-only` carries the state.
    Refused {
        reason: String,
        error: String,
        state: Option<Box<BleState>>,
    },
    /// The request breaks the protocol's rules, so nothing was sent.
    Invalid(&'static str),
}

impl BleError {
    pub fn reason(&self) -> Option<&str> {
        match self {
            BleError::Refused { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

impl fmt::Display for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BleError::Link(error) => error.fmt(f),
            BleError::Refused { reason, error, .. } if reason.is_empty() => {
                write!(f, "the Device refused: {error}")
            }
            BleError::Refused { reason, error, .. } => {
                write!(f, "the Device refused: {error} ({reason})")
            }
            BleError::Invalid(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for BleError {}

impl From<LinkError> for BleError {
    fn from(error: LinkError) -> Self {
        BleError::Link(error)
    }
}

fn invalid_reply() -> BleError {
    BleError::Link(LinkError::Reply(td_core::ReplyError::Fields(
        "the Device sent an invalid Bluetooth state".into(),
    )))
}

fn parse_state(fields: &Map<String, Value>) -> Option<BleState> {
    let state: BleState = serde_json::from_value(Value::Object(fields.clone())).ok()?;
    state.valid().then_some(state)
}

fn state(reply: &Reply) -> Result<BleState, BleError> {
    if !reply.ok {
        let text = |key: &str| {
            reply
                .fields
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return Err(BleError::Refused {
            reason: text("reason"),
            error: text("error"),
            state: parse_state(&reply.fields).map(Box::new),
        });
    }
    parse_state(&reply.fields).ok_or_else(invalid_reply)
}

fn call(session: &Session, request: Value, timeout: Duration) -> Result<Reply, BleError> {
    Ok(session.call(&request, timeout)?)
}

pub fn ble_state(session: &Session) -> Result<BleState, BleError> {
    state(&call(session, json!({"m": "ble.state"}), REQUEST_TIMEOUT)?)
}

/// Stores the setting, then starts or stops Bluetooth. A start may answer `starting`; poll [`ble_state`].
pub fn ble_set(session: &Session, enabled: bool) -> Result<BleState, BleError> {
    state(&call(
        session,
        json!({"m": "ble.set", "enabled": enabled}),
        SET_TIMEOUT,
    )?)
}

/// Opens the pairing window for [`PAIRING_MS`], or closes it.
pub fn ble_pair(session: &Session, open: bool) -> Result<BleState, BleError> {
    state(&call(
        session,
        json!({"m": "ble.pair", "open": open}),
        REQUEST_TIMEOUT,
    )?)
}

/// Forgets the bonded host at `address`, in either case, or every one when `None`. The address goes to the Device in
/// lower case.
pub fn ble_forget(session: &Session, address: Option<&str>) -> Result<BleForgotten, BleError> {
    let request = match address.map(|address| normalize_address(address).ok_or(())) {
        Some(Err(())) => {
            return Err(BleError::Invalid(
                "A Bluetooth address is six hex bytes, aa:bb:cc:dd:ee:ff.",
            ));
        }
        Some(Ok(address)) => json!({"m": "ble.forget", "address": address}),
        None => json!({"m": "ble.forget"}),
    };
    let reply = call(session, request, REQUEST_TIMEOUT)?;
    let state = state(&reply)?;
    let forgotten = reply
        .fields
        .get("forgotten")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= BONDS_MAX)
        .ok_or_else(invalid_reply)?;
    Ok(BleForgotten { forgotten, state })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MockDeck, SessionConfig};

    const PHONE: &str = "00:00:5e:00:53:01";
    const LAPTOP: &str = "00:00:5e:00:53:02";

    fn session(deck: &MockDeck) -> Session {
        let (session, _) = Session::open(
            Box::new(deck.connect()),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .unwrap();
        session.hello(Duration::from_secs(1)).unwrap();
        session
    }

    fn radio() -> MockDeck {
        let deck = MockDeck::v2();
        deck.set_ble_capability(Some(4));
        deck
    }

    #[test]
    fn addresses_follow_the_protocol() {
        assert!(valid_address(PHONE));
        assert!(valid_address("ff:ff:ff:ff:ff:ff"));
        assert!(!valid_address("00:00:5E:00:53:01"));
        assert!(!valid_address("00-00-5e-00-53-01"));
        assert!(!valid_address("00:00:5e:00:53"));
        assert!(!valid_address("00:00:5e:00:53:01:02"));
        assert!(!valid_address("0:00:5e:00:53:01a"));
        assert!(!valid_address("g0:00:5e:00:53:01"));
        assert!(!valid_address(""));
        assert_eq!(
            normalize_address("00:00:5E:00:53:0A").as_deref(),
            Some("00:00:5e:00:53:0a")
        );
        assert_eq!(normalize_address(PHONE).as_deref(), Some(PHONE));
        assert_eq!(normalize_address("00-00-5E-00-53-01"), None);
    }

    #[test]
    fn the_v2_hello_advertises_the_keyboard() {
        let deck = radio();
        let s = session(&deck);
        let identity = s.identity(Duration::from_secs(1)).unwrap();
        assert_eq!(identity.capabilities.unwrap().ble_hid, Some(4));

        let plain = MockDeck::v2();
        let s = session(&plain);
        let identity = s.identity(Duration::from_secs(1)).unwrap();
        assert_eq!(identity.capabilities.unwrap().ble_hid, None);
    }

    #[test]
    fn off_by_default_then_on_pair_and_off() {
        let deck = radio();
        let s = session(&deck);
        let off = ble_state(&s).unwrap();
        assert!(!off.enabled);
        assert_eq!(off.state, BleStack::Off);
        assert_eq!(off.advertising, BleAdvertising::Off);
        assert_eq!((off.bonds_max, off.battery), (4, 100));
        assert!(off.bonds.is_empty());

        // As the Firmware: the start answers `starting`, and the host has synced by the next request.
        let starting = ble_set(&s, true).unwrap();
        assert!(starting.enabled);
        assert_eq!(starting.state, BleStack::Starting);
        assert_eq!(ble_state(&s).unwrap().state, BleStack::On);

        let open = ble_pair(&s, true).unwrap();
        assert!(open.pairing);
        assert!(open.pairing_ms > PAIRING_MS - 2_000 && open.pairing_ms <= PAIRING_MS);
        assert_eq!(open.advertising, BleAdvertising::Pairing);
        assert!(!open.passkey);

        deck.ble_show_passkey(true);
        let showing = ble_state(&s).unwrap();
        assert!(showing.passkey);
        assert!(deck.ble_host_pairs(PHONE, false));
        let paired = ble_state(&s).unwrap();
        assert!(!paired.pairing, "a new bond closes the window");
        assert!(!paired.passkey);
        assert_eq!(
            paired.bonds,
            [BleBond {
                address: PHONE.into(),
                random: false,
                connected: true
            }]
        );

        let closed = ble_pair(&s, false).unwrap();
        assert_eq!((closed.pairing, closed.pairing_ms), (false, 0));

        let off = ble_set(&s, false).unwrap();
        assert_eq!(off.state, BleStack::Off);
        assert!(off.bonds.is_empty(), "a Device that is off lists no bonds");
        assert_eq!(deck.ble_bonds().len(), 1, "turning off keeps the bonds");
        // What went over the link names no passkey.
        let received = serde_json::to_string(&deck.received()).unwrap();
        assert!(!received.contains("passkey\":"));
    }

    #[test]
    fn forget_one_then_all() {
        let deck = radio();
        deck.add_ble_bond(PHONE, false, true);
        deck.add_ble_bond(LAPTOP, true, false);
        let s = session(&deck);
        ble_set(&s, true).unwrap();
        assert_eq!(ble_state(&s).unwrap().bonds.len(), 2);

        // Either case: the Firmware takes upper case too, and the request carries lower case.
        let one = ble_forget(&s, Some(&PHONE.to_ascii_uppercase())).unwrap();
        let sent = deck.received();
        assert_eq!(
            sent.iter().rev().find(|r| r["m"] == "ble.forget").unwrap()["address"],
            PHONE
        );
        assert_eq!(one.forgotten, 1);
        assert_eq!(one.state.bonds.len(), 1);
        assert_eq!(one.state.bonds[0].address, LAPTOP);
        assert!(one.state.bonds[0].random);

        let none = ble_forget(&s, Some(PHONE)).unwrap();
        assert_eq!(none.forgotten, 0);

        deck.add_ble_bond(PHONE, false, false);
        let all = ble_forget(&s, None).unwrap();
        assert_eq!(all.forgotten, 2);
        assert!(all.state.bonds.is_empty());
        assert!(deck.ble_bonds().is_empty());
    }

    #[test]
    fn refusals_carry_the_state() {
        let deck = radio();
        let s = session(&deck);
        let error = ble_pair(&s, true).unwrap_err();
        assert_eq!(error.reason(), Some("off"));
        let BleError::Refused {
            state: Some(state), ..
        } = error
        else {
            panic!("an off refusal carries the state");
        };
        assert_eq!(state.state, BleStack::Off);
        assert_eq!(ble_forget(&s, None).unwrap_err().reason(), Some("off"));

        for address in [
            "00:00:5e:00:53:01",
            "00:00:5e:00:53:02",
            "00:00:5e:00:53:03",
            "00:00:5e:00:53:04",
        ] {
            deck.add_ble_bond(address, false, false);
        }
        ble_set(&s, true).unwrap();
        assert_eq!(ble_pair(&s, true).unwrap_err().reason(), Some("full"));

        deck.fail_ble_start(true);
        ble_set(&s, false).unwrap();
        let error = ble_set(&s, true).unwrap_err();
        assert_eq!(error.reason(), Some("radio"));
        let BleError::Refused {
            state: Some(state), ..
        } = error
        else {
            panic!("a radio refusal carries the state");
        };
        assert!(state.enabled, "the setting stays on");
        assert_eq!(state.state, BleStack::Failed);
    }

    #[test]
    fn a_device_without_the_keyboard_does_not_know_the_messages() {
        let deck = MockDeck::v2();
        let s = session(&deck);
        let error = ble_state(&s).unwrap_err();
        assert_eq!(error.reason(), Some(""));
        assert!(matches!(error, BleError::Refused { state: None, .. }));
        let before = deck.received().len();
        assert!(matches!(
            ble_forget(&s, Some("00-00-5e-00-53-01")),
            Err(BleError::Invalid(_))
        ));
        assert_eq!(deck.received().len(), before);
    }

    #[test]
    fn bad_fields_are_args() {
        let deck = radio();
        let s = session(&deck);
        for request in [
            json!({"m": "ble.state", "extra": true}),
            json!({"m": "ble.set"}),
            json!({"m": "ble.set", "enabled": "yes"}),
            json!({"m": "ble.pair", "open": 1}),
            json!({"m": "ble.forget", "address": "00-00-5e-00-53-01"}),
            json!({"m": "ble.forget", "address": 1}),
        ] {
            let reply = s.call(&request, REQUEST_TIMEOUT).unwrap();
            assert!(!reply.ok, "{request}");
            assert_eq!(reply.fields["reason"], "args", "{request}");
        }
    }

    #[test]
    fn a_host_reset_says_starting_and_keeps_listing_the_bonds() {
        let deck = radio();
        deck.add_ble_bond(PHONE, false, true);
        let s = session(&deck);
        ble_set(&s, true).unwrap();
        assert_eq!(ble_state(&s).unwrap().bonds.len(), 1);

        deck.ble_hold_start(true);
        deck.ble_host_reset();
        let starting = ble_state(&s).unwrap();
        assert_eq!(starting.state, BleStack::Starting);
        assert_eq!(starting.bonds.len(), 1, "bonds may be listed in any state");
        assert!(!starting.bonds[0].connected);
        assert_eq!(ble_pair(&s, true).unwrap_err().reason(), Some("starting"));
        let error = ble_forget(&s, None).unwrap_err();
        assert_eq!(error.reason(), Some("starting"));
        assert!(matches!(error, BleError::Refused { state: Some(_), .. }));

        deck.ble_hold_start(false);
        assert_eq!(ble_state(&s).unwrap().state, BleStack::On);
    }

    #[test]
    fn a_failed_start_then_off_reads_off() {
        let deck = radio();
        deck.fail_ble_start(true);
        let s = session(&deck);
        assert_eq!(ble_set(&s, true).unwrap_err().reason(), Some("radio"));
        let off = ble_set(&s, false).unwrap();
        assert_eq!((off.enabled, off.state), (false, BleStack::Off));
        assert_eq!(ble_state(&s).unwrap().state, BleStack::Off);
    }

    #[test]
    fn storage_refusals_carry_the_state() {
        let deck = radio();
        deck.add_ble_bond(PHONE, false, false);
        let s = session(&deck);
        deck.fail_ble_storage(true);
        let error = ble_set(&s, true).unwrap_err();
        assert_eq!(error.reason(), Some("storage"));
        assert!(
            matches!(error, BleError::Refused { state: Some(ref state), .. } if !state.enabled)
        );
        assert!(!deck.ble_enabled());

        deck.fail_ble_storage(false);
        ble_set(&s, true).unwrap();
        ble_state(&s).unwrap();
        deck.fail_ble_storage(true);
        let error = ble_forget(&s, Some(PHONE)).unwrap_err();
        assert_eq!(error.reason(), Some("storage"));
        assert_eq!(deck.ble_bonds().len(), 1);
    }

    #[test]
    fn a_radio_build_answers_an_unknown_ble_message_unsupported() {
        let deck = radio();
        let s = session(&deck);
        let reply = s.call(&json!({"m": "ble.scan"}), REQUEST_TIMEOUT).unwrap();
        assert!(!reply.ok);
        assert_eq!(reply.fields["reason"], "unsupported");
        assert!(!reply.fields.contains_key("state"));
    }
}
