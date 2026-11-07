//! The apps a radio-build Device keeps Pairings with, as `schema/protocol/net-v2.md` § "Managing Pairings" defines
//! them.
//!
//! `net.list` lists them, oldest first, by app_id and name. `net.forget` forgets one by app_id, or every one with
//! `all:true`, which also clears the failed-code counter and the lockout. `net.pair` opens the pairing window, as
//! "Pair new app" does on the Device. All three are USB-only: over any other Carrier the Device refuses them with
//! `usb-only` and changes nothing. No answer carries a key.

use std::fmt;
use std::time::Duration;

use serde_json::{Value, json};
use td_core::Reply;

use crate::net::{CompanionId, parse_hex};
use crate::session::{LinkError, Session};

/// The most Pairings a Device keeps.
pub const MAX_PAIRINGS: u32 = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// `net.forget` erases flash and ends a session before it answers.
const FORGET_TIMEOUT: Duration = Duration::from_secs(5);

/// One app the Device keeps a Pairing with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetPairing {
    /// The Companion id the app paired with.
    pub app_id: CompanionId,
    /// The name the app gave when it paired.
    pub app_name: String,
}

/// What `net.list` answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetPairings {
    /// Oldest first.
    pub pairings: Vec<NetPairing>,
    /// Slots left, `MAX_PAIRINGS` less the Pairings.
    pub free: u32,
    /// Whether a pairing window is open.
    pub pairing: bool,
    /// Wrong codes since the last good pairing.
    pub pair_failures: u32,
    /// Milliseconds left in a pairing lockout, 0 when none.
    pub locked_ms: u64,
    /// The app whose network session runs now.
    pub session: Option<CompanionId>,
}

/// What to forget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forget {
    One(CompanionId),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingsError {
    Link(LinkError),
    /// The Device refused with this stable reason: `usb-only`, `full`, `locked`, `args`, `storage`, or none for a
    /// Device that doesn't know `net.*`.
    Refused {
        reason: String,
        error: String,
    },
}

impl PairingsError {
    pub fn reason(&self) -> Option<&str> {
        match self {
            PairingsError::Refused { reason, .. } => Some(reason),
            PairingsError::Link(_) => None,
        }
    }
}

impl fmt::Display for PairingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairingsError::Link(error) => error.fmt(f),
            PairingsError::Refused { reason, error } if reason.is_empty() => {
                write!(f, "the Device refused: {error}")
            }
            PairingsError::Refused { reason, error } => {
                write!(f, "the Device refused: {error} ({reason})")
            }
        }
    }
}

impl std::error::Error for PairingsError {}

impl From<LinkError> for PairingsError {
    fn from(error: LinkError) -> Self {
        PairingsError::Link(error)
    }
}

/// An app_id as the Device writes it: 32 lower-case hex characters.
pub fn app_id_hex(app_id: &CompanionId) -> String {
    app_id.iter().map(|b| format!("{b:02x}")).collect()
}

/// An app_id in lower-case hex, as `net.forget` takes it. `None` for anything else.
pub fn parse_app_id(text: &str) -> Option<CompanionId> {
    if text.len() != 32 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    parse_hex(text)
}

fn invalid_reply() -> PairingsError {
    PairingsError::Link(LinkError::Reply(td_core::ReplyError::Fields(
        "the Device sent an invalid Pairing list".into(),
    )))
}

fn answered(reply: Reply) -> Result<Reply, PairingsError> {
    if reply.ok {
        return Ok(reply);
    }
    let text = |key: &str| {
        reply
            .fields
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Err(PairingsError::Refused {
        reason: text("reason"),
        error: text("error"),
    })
}

fn number(reply: &Reply, key: &str) -> Option<u64> {
    reply.fields.get(key).and_then(Value::as_u64)
}

fn parse_list(reply: &Reply) -> Option<NetPairings> {
    let pairings = reply
        .fields
        .get("pairings")?
        .as_array()?
        .iter()
        .map(|entry| {
            let app_id = parse_app_id(entry.get("app_id")?.as_str()?)?;
            let app_name = entry.get("app_name")?.as_str()?.to_owned();
            Some(NetPairing { app_id, app_name })
        })
        .collect::<Option<Vec<_>>>()?;
    let free = u32::try_from(number(reply, "free")?).ok()?;
    if pairings.len() as u32 + free != MAX_PAIRINGS {
        return None;
    }
    let session = match reply.fields.get("session")? {
        Value::Null => None,
        Value::String(text) => Some(parse_app_id(text)?),
        _ => return None,
    };
    Some(NetPairings {
        pairings,
        free,
        pairing: reply.fields.get("pairing")?.as_bool()?,
        pair_failures: u32::try_from(number(reply, "pair_failures")?).ok()?,
        locked_ms: number(reply, "locked_ms")?,
        session,
    })
}

/// Lists the apps the Device keeps Pairings with.
pub fn net_list(session: &Session) -> Result<NetPairings, PairingsError> {
    let reply = answered(session.call(&json!({"m": "net.list"}), REQUEST_TIMEOUT)?)?;
    parse_list(&reply).ok_or_else(invalid_reply)
}

/// Forgets one app, or every one. Forgetting an app the Device doesn't keep succeeds and removes none. Answers how
/// many the Device removed.
pub fn net_forget(session: &Session, forget: Forget) -> Result<u32, PairingsError> {
    let request = match forget {
        Forget::One(app_id) => json!({"m": "net.forget", "app_id": app_id_hex(&app_id)}),
        Forget::All => json!({"m": "net.forget", "all": true}),
    };
    let reply = answered(session.call(&request, FORGET_TIMEOUT)?)?;
    number(&reply, "removed")
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= MAX_PAIRINGS)
        .ok_or_else(invalid_reply)
}

/// Opens the Device's pairing window, as "Pair new app" on its screen does.
pub fn net_pair(session: &Session) -> Result<(), PairingsError> {
    answered(session.call(&json!({"m": "net.pair"}), REQUEST_TIMEOUT)?).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MockDeck, SessionConfig};

    const ONE: CompanionId = [0x10; 16];
    const TWO: CompanionId = [0x20; 16];

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
        deck.set_wifi_capability(true);
        deck
    }

    #[test]
    fn lists_oldest_first_and_forgets_one() {
        let deck = radio();
        deck.add_net_pairing(TWO, "Laptop");
        deck.add_net_pairing(ONE, "Desk PC");
        let s = session(&deck);
        let listed = net_list(&s).unwrap();
        assert_eq!(listed.free, 2);
        assert_eq!(
            listed.pairings,
            vec![
                NetPairing {
                    app_id: TWO,
                    app_name: "Laptop".into()
                },
                NetPairing {
                    app_id: ONE,
                    app_name: "Desk PC".into()
                },
            ]
        );
        assert_eq!(net_forget(&s, Forget::One(TWO)).unwrap(), 1);
        // Idempotent: the same request again removes none.
        assert_eq!(net_forget(&s, Forget::One(TWO)).unwrap(), 0);
        assert_eq!(net_list(&s).unwrap().pairings.len(), 1);
        assert_eq!(net_forget(&s, Forget::All).unwrap(), 1);
        assert!(net_list(&s).unwrap().pairings.is_empty());
    }

    #[test]
    fn a_full_device_refuses_a_window_until_one_goes() {
        let deck = radio();
        for n in 0..4u8 {
            deck.add_net_pairing([n; 16], &format!("App {n}"));
        }
        let s = session(&deck);
        let full = net_pair(&s).unwrap_err();
        assert_eq!(full.reason(), Some("full"));
        net_forget(&s, Forget::One([0; 16])).unwrap();
        net_pair(&s).unwrap();
        assert!(net_list(&s).unwrap().pairing);
    }

    #[test]
    fn a_failed_erase_answers_storage() {
        let deck = radio();
        deck.add_net_pairing(ONE, "Desk PC");
        deck.fail_net_storage(true);
        let s = session(&deck);
        let error = net_forget(&s, Forget::One(ONE)).unwrap_err();
        assert_eq!(error.reason(), Some("storage"));
    }

    #[test]
    fn the_normal_build_does_not_know_them() {
        let deck = MockDeck::v2();
        let s = session(&deck);
        let error = net_list(&s).unwrap_err();
        assert_eq!(error.reason(), Some(""));
    }

    #[test]
    fn app_ids_are_lower_case_hex() {
        assert_eq!(parse_app_id(&app_id_hex(&ONE)), Some(ONE));
        assert_eq!(parse_app_id("101112131415161718191A1B1C1D1E1F"), None);
        assert_eq!(parse_app_id("+0101112131415161718191a1b1c1d1e"), None);
        assert_eq!(parse_app_id("1011"), None);
    }
}
