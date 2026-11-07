//! The Device's Wi-Fi network, as `schema/protocol/link-v2.md` § "Wi-Fi network" defines it.
//!
//! Only the radio build has a Wi-Fi station, and it says so with `caps.wifi`. `wifi.set {ssid, password}` stores one
//! network and joins it, `wifi.state` reads how the join went, and `wifi.forget` erases it. All three are USB-only.
//!
//! The password goes to the Device and never comes back. [`WifiPassword`] keeps it out of `Debug`, and nothing here
//! prints, logs or returns it.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use td_core::Reply;

use crate::session::{LinkError, Session};

pub const SSID_MAX: usize = 32;
pub const PASSWORD_MIN: usize = 8;
/// A passphrase is 8 to 63 printable ASCII characters; 64 hex digits are a raw key.
pub const PASSWORD_MAX: usize = 64;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// A Wi-Fi password. Its `Debug` shows only its length class, and it has no `Display` or `Serialize`.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct WifiPassword(String);

impl WifiPassword {
    pub fn new(password: impl Into<String>) -> Self {
        WifiPassword(password.into())
    }

    /// The password, for the one request that sends it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for WifiPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() {
            "WifiPassword(empty)"
        } else {
            "WifiPassword(redacted)"
        })
    }
}

/// Printable ASCII, 0x20 to 0x7e: what IEEE 802.11 allows in a WPA passphrase.
fn printable_ascii(text: &str) -> bool {
    text.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

fn text_valid(text: &str) -> bool {
    !text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// 1 to 32 UTF-8 bytes without control characters.
pub fn valid_ssid(ssid: &str) -> bool {
    !ssid.is_empty() && ssid.len() <= SSID_MAX && text_valid(ssid)
}

/// Empty for an open network, 8 to 63 printable ASCII characters, or 64 hex digits.
pub fn valid_password(password: &WifiPassword) -> bool {
    let p = password.expose();
    p.is_empty()
        || ((PASSWORD_MIN..PASSWORD_MAX).contains(&p.len()) && printable_ascii(p))
        || (p.len() == PASSWORD_MAX && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// How the station is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiStatus {
    /// No network is stored.
    Off,
    Connecting,
    Connected,
    /// The last join failed or the connection dropped; the Device keeps retrying.
    Failed,
}

/// What the Device reports: never the password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiState {
    pub configured: bool,
    pub ssid: String,
    pub state: WifiStatus,
    /// Why the station failed, such as `not-found` or `wrong-password`. `None` unless `state` is `failed`.
    pub reason: Option<String>,
    pub ip: Option<String>,
    pub rssi: Option<i32>,
}

impl WifiState {
    fn valid(&self) -> bool {
        (self.configured == (self.state != WifiStatus::Off))
            && (self.configured == !self.ssid.is_empty())
            && self.ssid.len() <= SSID_MAX
            && (self.state == WifiStatus::Failed) == self.reason.is_some()
            && (self.state == WifiStatus::Connected || (self.ip.is_none() && self.rssi.is_none()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiError {
    Link(LinkError),
    /// The Device refused with this stable reason: `usb-only`, `unsupported`, `args`, `storage`, ...
    Refused {
        reason: String,
        error: String,
    },
    /// The SSID or the password breaks the protocol's rules, so nothing was sent.
    Invalid(&'static str),
}

impl WifiError {
    pub fn reason(&self) -> Option<&str> {
        match self {
            WifiError::Refused { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

impl fmt::Display for WifiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WifiError::Link(error) => error.fmt(f),
            WifiError::Refused { reason, error } => {
                write!(f, "the Device refused: {error} ({reason})")
            }
            WifiError::Invalid(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for WifiError {}

impl From<LinkError> for WifiError {
    fn from(error: LinkError) -> Self {
        WifiError::Link(error)
    }
}

fn state(reply: &Reply) -> Result<WifiState, WifiError> {
    if !reply.ok {
        let text = |key: &str| {
            reply
                .fields
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return Err(WifiError::Refused {
            reason: text("reason"),
            error: text("error"),
        });
    }
    let invalid = || {
        WifiError::Link(LinkError::Reply(td_core::ReplyError::Fields(
            "the Device sent an invalid Wi-Fi state".into(),
        )))
    };
    let state: WifiState =
        serde_json::from_value(Value::Object(reply.fields.clone())).map_err(|_| invalid())?;
    if !state.valid() {
        return Err(invalid());
    }
    Ok(state)
}

fn call(session: &Session, name: &str, payload: Vec<u8>) -> Result<WifiState, WifiError> {
    state(&session.call_payload(name, payload, REQUEST_TIMEOUT)?)
}

pub fn wifi_state(session: &Session) -> Result<WifiState, WifiError> {
    call(session, "wifi.state", br#"{"m":"wifi.state"}"#.to_vec())
}

/// Stores the network on the Device and starts joining it. The reply says `connecting`; poll [`wifi_state`] for the
/// outcome.
pub fn wifi_set(
    session: &Session,
    ssid: &str,
    password: &WifiPassword,
) -> Result<WifiState, WifiError> {
    #[derive(Serialize)]
    struct Set<'a> {
        m: &'static str,
        ssid: &'a str,
        password: &'a str,
    }
    if !valid_ssid(ssid) {
        return Err(WifiError::Invalid(
            "The network name is 1 to 32 bytes without control characters.",
        ));
    }
    if !valid_password(password) {
        return Err(WifiError::Invalid(
            "The password is empty for an open network, 8 to 63 printable ASCII characters, or 64 hex digits.",
        ));
    }
    let payload = serde_json::to_vec(&Set {
        m: "wifi.set",
        ssid,
        password: password.expose(),
    })
    .expect("Wi-Fi requests serialize");
    call(session, "wifi.set", payload)
}

pub fn wifi_forget(session: &Session) -> Result<WifiState, WifiError> {
    call(session, "wifi.forget", br#"{"m":"wifi.forget"}"#.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MockDeck, SessionConfig};

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

    #[test]
    fn passwords_follow_the_protocol() {
        assert!(valid_password(&WifiPassword::new("")));
        assert!(valid_password(&WifiPassword::new("12345678")));
        assert!(!valid_password(&WifiPassword::new("1234567")));
        assert!(valid_password(&WifiPassword::new("a".repeat(63))));
        assert!(!valid_password(&WifiPassword::new("g".repeat(64))));
        assert!(valid_password(&WifiPassword::new("F".repeat(64))));
        assert!(!valid_password(&WifiPassword::new("line\nbreak")));
        // A passphrase is printable ASCII only.
        assert!(valid_password(&WifiPassword::new(" ~dummy pass~ ")));
        assert!(!valid_password(&WifiPassword::new("dummy-pass\u{7f}")));
        assert!(!valid_password(&WifiPassword::new("café-dummy-pass")));
        assert!(!valid_password(&WifiPassword::new("dummy-pass-\u{200b}")));
        assert!(valid_ssid("td-test-network"));
        assert!(valid_ssid(&"x".repeat(32)));
        assert!(!valid_ssid(&"x".repeat(33)));
        assert!(!valid_ssid(""));
        assert!(!valid_ssid("tab\there"));
    }

    #[test]
    fn debug_never_shows_the_password() {
        let password = WifiPassword::new("dummy-pass-1234");
        let shown = format!("{password:?} {:?}", Some(&password));
        assert!(!shown.contains("dummy-pass-1234"));
        assert_eq!(format!("{password:?}"), "WifiPassword(redacted)");
    }

    #[test]
    fn set_join_and_forget_on_a_radio_device() {
        let deck = MockDeck::v2();
        deck.set_wifi_capability(true);
        deck.add_wifi_network("td-test-network", "dummy-pass-1234");
        let s = session(&deck);
        let off = wifi_state(&s).unwrap();
        assert_eq!(off.state, WifiStatus::Off);
        assert!(!off.configured);

        let set = wifi_set(&s, "td-test-network", &WifiPassword::new("dummy-pass-1234")).unwrap();
        assert_eq!(set.state, WifiStatus::Connecting);
        assert_eq!(set.ssid, "td-test-network");
        let joined = wifi_state(&s).unwrap();
        assert_eq!(joined.state, WifiStatus::Connected);
        assert!(joined.ip.as_deref().unwrap().starts_with("192.0.2."));
        assert_eq!(
            deck.wifi_network(),
            Some(("td-test-network".into(), "dummy-pass-1234".into()))
        );
        // What the mock records never holds the password.
        let received = serde_json::to_string(&deck.received()).unwrap();
        assert!(!received.contains("dummy-pass-1234"));

        wifi_set(&s, "td-test-network", &WifiPassword::new("wrong-pass-5678")).unwrap();
        let wrong = wifi_state(&s).unwrap();
        assert_eq!(wrong.state, WifiStatus::Failed);
        assert_eq!(wrong.reason.as_deref(), Some("wrong-password"));

        wifi_set(
            &s,
            "td-test-nonexistent",
            &WifiPassword::new("dummy-pass-1234"),
        )
        .unwrap();
        assert_eq!(wifi_state(&s).unwrap().reason.as_deref(), Some("not-found"));

        let forgot = wifi_forget(&s).unwrap();
        assert_eq!(forgot.state, WifiStatus::Off);
        assert_eq!(deck.wifi_network(), None);
    }

    #[test]
    fn a_device_without_wifi_refuses_and_bad_input_stays_here() {
        let deck = MockDeck::v2();
        let s = session(&deck);
        let error = wifi_state(&s).unwrap_err();
        assert_eq!(error.reason(), Some("unsupported"));
        let before = deck.received().len();
        assert!(matches!(
            wifi_set(&s, "", &WifiPassword::new("dummy-pass-1234")),
            Err(WifiError::Invalid(_))
        ));
        assert!(matches!(
            wifi_set(&s, "td-test-network", &WifiPassword::new("short")),
            Err(WifiError::Invalid(_))
        ));
        assert_eq!(deck.received().len(), before);
    }

    #[test]
    fn usb_only_refusals_come_back_as_refused() {
        let deck = MockDeck::v2();
        deck.set_wifi_capability(true);
        deck.set_wifi_usb_only(true);
        let s = session(&deck);
        let error =
            wifi_set(&s, "td-test-network", &WifiPassword::new("dummy-pass-1234")).unwrap_err();
        assert_eq!(error.reason(), Some("usb-only"));
        assert_eq!(deck.wifi_network(), None);
    }
}
