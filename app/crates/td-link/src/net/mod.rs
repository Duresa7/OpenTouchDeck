//! The Wi-Fi carrier: net-v2 (`schema/protocol/net-v2.md`) over TCP. A Companion pairs with a
//! Device once, by the code the Device shows, and keeps the long-term key it gets. Each session
//! then runs an X25519 exchange and carries link frames in AES-256-GCM records, so the link
//! [`Session`](crate::Session) runs over it unchanged.
//!
//! [`pair`] starts pairing at an address, [`NetCarrier::connect`] opens a session with a
//! [`Credential`], and [`mdns::browse`] finds Devices on the LAN. With the `fake` feature,
//! `fake::FakeNetDevice` is a Device on loopback for tests; release builds leave it out.

mod client;
#[cfg(feature = "fake")]
pub mod fake;
pub mod mdns;
mod socket;
pub mod wire;

use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub use client::{NetCarrier, Paired, Pairing, pair};
use zeroize::Zeroize;

/// The TCP port a Device listens on.
pub const PORT: u16 = 47_820;
/// The DNS-SD service type Devices announce.
pub const SERVICE: &str = "_touchdeck._tcp.local.";
/// How long a session's setup may take, from the TCP connection.
pub const SESSION_SETUP: Duration = Duration::from_secs(10);
/// How long a pairing attempt may take, from `pair.start`.
pub const PAIR_ATTEMPT: Duration = Duration::from_secs(10);
/// Failed codes a pairing window takes before the Device closes it.
pub const WINDOW_FAILURES: u8 = 3;

/// A handshake error the Device sent. These are hints: they never prove who sent them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Refusal {
    Busy = 1,
    Version = 2,
    Args = 3,
    Full = 4,
    Auth = 5,
    Expired = 6,
    Cancelled = 7,
    Storage = 8,
    Closed = 9,
}

impl Refusal {
    pub fn from_u8(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Busy,
            2 => Self::Version,
            3 => Self::Args,
            4 => Self::Full,
            5 => Self::Auth,
            6 => Self::Expired,
            7 => Self::Cancelled,
            8 => Self::Storage,
            9 => Self::Closed,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Version => "version",
            Self::Args => "args",
            Self::Full => "full",
            Self::Auth => "auth",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
            Self::Storage => "storage",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug)]
pub enum NetError {
    /// The Device answered with a handshake error.
    Refused(Refusal),
    /// A handshake item with an unsupported version.
    Version(u8),
    /// The peer broke the protocol: a malformed item, an unexpected kind or a bad length.
    Protocol(String),
    /// A proof, tag or key check failed.
    Auth(String),
    /// The Device's id isn't the one this Companion paired with.
    WrongDevice,
    /// The connection ended.
    Closed,
    /// The pending key couldn't be saved, so pairing stopped before `pair.commit`.
    Store(String),
    Io(io::Error),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Refused(reason) => write!(f, "the Device refused ({})", reason.as_str()),
            NetError::Version(v) => write!(f, "the Device sent net item version {v}"),
            NetError::Protocol(detail) => write!(f, "net-v2 protocol error: {detail}"),
            NetError::Auth(detail) => write!(f, "net-v2 authentication failed: {detail}"),
            NetError::WrongDevice => f.write_str("the Device's id isn't the paired one"),
            NetError::Closed => f.write_str("the Device closed the connection"),
            NetError::Store(detail) => write!(f, "the pairing key couldn't be saved: {detail}"),
            NetError::Io(error) => write!(f, "network error: {error}"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<io::Error> for NetError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe => NetError::Closed,
            _ => NetError::Io(error),
        }
    }
}

impl NetError {
    /// Whether the connection or the deadline ran out, rather than a check failing.
    pub fn is_timeout(&self) -> bool {
        matches!(self, NetError::Io(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock))
    }
}

/// A 6-byte Device id: the factory base MAC, the link-v2 `id` and the USB serial.
pub type DeviceId = [u8; wire::DEVICE_ID_LEN];
/// A random 16-byte id, stable for one Companion installation.
pub type CompanionId = [u8; wire::COMPANION_ID_LEN];

/// The 12 lowercase hex digits of a Device id.
pub fn device_hex(id: &DeviceId) -> String {
    wire::hex(id)
}

/// Parses 12 hex digits, with or without `:` separators.
pub fn parse_device_id(text: &str) -> Option<DeviceId> {
    let digits: String = text.chars().filter(|c| *c != ':').collect();
    parse_hex(&digits)
}

/// Parses exactly `N` bytes of hex.
pub fn parse_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// What a pairing leaves on the Companion: which Device, as which Companion, with which key.
/// The key is erased when the credential drops.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    pub device_id: DeviceId,
    pub companion_id: CompanionId,
    pub ltk: [u8; wire::LTK_LEN],
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("device_id", &device_hex(&self.device_id))
            .field("companion_id", &wire::hex(&self.companion_id))
            .finish_non_exhaustive()
    }
}

impl Drop for Credential {
    fn drop(&mut self) {
        self.ltk.zeroize();
    }
}

/// Where a Device listens: an IPv4 or IPv6 address or a host name, and a port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address {
    pub host: String,
    pub port: u16,
}

impl Address {
    /// Parses what a person typed: `192.0.2.10`, `192.0.2.10:47820`, `2001:db8::5`,
    /// `[2001:db8::5]:47820` or `touchdeck.local:47820`. The port defaults to 47820.
    pub fn parse(text: &str) -> Option<Address> {
        let text = text.trim();
        if text.is_empty()
            || text.len() > 255
            || text.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return None;
        }
        if let Ok(ip) = text.parse::<IpAddr>() {
            return Some(Address {
                host: ip.to_string(),
                port: PORT,
            });
        }
        if let Ok(addr) = text.parse::<SocketAddr>() {
            return (addr.port() != 0).then(|| Address::from(addr));
        }
        if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            let ip: std::net::Ipv6Addr = inner.parse().ok()?;
            return Some(Address {
                host: ip.to_string(),
                port: PORT,
            });
        }
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().ok().filter(|p| *p != 0)?),
            None => (text, PORT),
        };
        let host = host.trim_end_matches('.');
        let valid = !host.is_empty()
            && host.split('.').all(|label| {
                (1..=63).contains(&label.len())
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !label.starts_with('-')
                    && !label.ends_with('-')
            });
        valid.then(|| Address {
            host: host.to_ascii_lowercase(),
            port,
        })
    }

    /// Connects within `timeout`, trying each resolved address in turn.
    pub fn connect(&self, timeout: Duration) -> Result<TcpStream, NetError> {
        let deadline = Instant::now() + timeout;
        let targets: Vec<SocketAddr> = (self.host.as_str(), self.port).to_socket_addrs()?.collect();
        let mut last = io::Error::new(io::ErrorKind::NotFound, "the address didn't resolve");
        for target in targets {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::from(io::ErrorKind::TimedOut).into());
            }
            match TcpStream::connect_timeout(&target, left) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    return Ok(stream);
                }
                Err(error) => last = error,
            }
        }
        Err(last.into())
    }
}

impl From<SocketAddr> for Address {
    fn from(addr: SocketAddr) -> Self {
        Address {
            host: addr.ip().to_string(),
            port: addr.port(),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

/// Fills `bytes` from the OS random source.
pub(crate) fn random(bytes: &mut [u8]) {
    getrandom::fill(bytes).expect("the OS random source works");
}

/// A random nonzero 32-byte value.
pub(crate) fn random_exponent() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    while bytes.iter().all(|b| *b == 0) {
        random(&mut bytes);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_parse_with_the_default_port() {
        let parse = |t| Address::parse(t).map(|a| a.to_string());
        assert_eq!(parse("192.0.2.10").as_deref(), Some("192.0.2.10:47820"));
        assert_eq!(
            parse(" 192.0.2.10:5000 ").as_deref(),
            Some("192.0.2.10:5000")
        );
        assert_eq!(parse("2001:db8::5").as_deref(), Some("[2001:db8::5]:47820"));
        assert_eq!(
            parse("[2001:db8::5]:4000").as_deref(),
            Some("[2001:db8::5]:4000")
        );
        assert_eq!(
            parse("[2001:db8::5]").as_deref(),
            Some("[2001:db8::5]:47820")
        );
        assert_eq!(
            parse("TouchDeck.local").as_deref(),
            Some("touchdeck.local:47820")
        );
        assert_eq!(parse("deck.example:81").as_deref(), Some("deck.example:81"));
        for bad in [
            "",
            "a b",
            "deck:0",
            "deck:99999",
            "-bad.example",
            "http://x",
            "x..y",
            "192.0.2.1:",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn device_ids_round_trip_as_hex() {
        let id = parse_device_id("02:00:00:00:00:01").unwrap();
        assert_eq!(device_hex(&id), "020000000001");
        assert_eq!(parse_device_id("020000000001"), Some(id));
        assert_eq!(parse_device_id("02000000000"), None);
        assert_eq!(parse_device_id("02000000000g"), None);
    }
}
