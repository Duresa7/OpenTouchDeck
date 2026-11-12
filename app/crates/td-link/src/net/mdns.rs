//! Finding Devices on the LAN with DNS-SD over multicast DNS (`_touchdeck._tcp`). This uses
//! `mdns-sd`, a pure-Rust querier, so it needs neither Bonjour nor Avahi and works next to an
//! OS responder that already owns port 5353. A browse runs only when asked and stops when it
//! returns. Where multicast is blocked it finds nothing, and the person types an address.
//!
//! Outside pairing mode a record names nothing: it carries a random nonce and four hints, and
//! only a Companion holding a Pairing's key recognises its hint ([`Advert::matches`]). In
//! pairing mode it also carries the Device id, name and Pairing count.
//!
//! Records are untrusted hints: a Pairing pins the Device id and every session checks it.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use super::wire::{ADVERT_NONCE_LEN, HINT_LEN};
use super::{Address, DeviceId, SERVICE, parse_device_id, wire};

/// The longest a browse may run.
pub const MAX_BROWSE: Duration = Duration::from_secs(10);
/// The most Devices one browse returns. A noisy LAN can't grow the list past this.
pub const MAX_FOUND: usize = 32;
/// The most answers one browse reads before it stops listening.
const MAX_ANSWERS: usize = 256;
/// The most Pairings a Device holds, and so the hints in a record.
pub const HINT_SLOTS: usize = 4;

/// What a Device in pairing mode says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingMode {
    pub device_id: DeviceId,
    pub name: String,
    /// Stored Pairings on the Device, when it said.
    pub paired: Option<u8>,
}

/// One Device that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advert {
    pub address: Address,
    pub nonce: [u8; ADVERT_NONCE_LEN],
    pub hints: [[u8; HINT_LEN]; HINT_SLOTS],
    /// Set only while a pairing window is open on the Device.
    pub pairing: Option<PairingMode>,
}

impl Advert {
    /// Whether this record carries the hint of the Pairing with this discovery key. Every
    /// hint is compared, in constant time.
    pub fn matches(&self, discovery_key: &[u8; 32]) -> bool {
        let mine = wire::hint(discovery_key, &self.nonce);
        self.hints
            .iter()
            .fold(false, |hit, hint| wire::same(&mine, hint) | hit)
    }
}

/// What a resolved DNS-SD instance carried, before any of it is trusted.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub txt: BTreeMap<String, String>,
}

/// Whether a LAN Device could be at `ip`. Loopback, link-local, unspecified, broadcast and
/// multicast addresses are dropped, so an answer can't steer pairing at this PC or at an
/// address that needs a scope.
fn usable(ip: &IpAddr) -> bool {
    if ip.is_unspecified() || ip.is_multicast() || ip.is_loopback() {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => !v4.is_link_local() && !v4.is_broadcast(),
        IpAddr::V6(v6) => {
            (v6.segments()[0] & 0xffc0) != 0xfe80
                && v6.to_ipv4_mapped().is_none_or(|v4| usable(&IpAddr::V4(v4)))
        }
    }
}

/// Lowercase hex of exactly `N` bytes, or `None`.
fn lower_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != 2 * N || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    super::parse_hex::<N>(text)
}

/// Reads one resolved instance. Anything that isn't net-v2 with a valid nonce, four hints and
/// a usable address is dropped. IPv4 is preferred, then IPv6.
pub fn read(resolved: &Resolved) -> Option<Advert> {
    read_where(resolved, usable)
}

/// [`read`] for the fake Devices of tests, which live on loopback.
#[cfg(feature = "fake")]
pub fn read_loopback(resolved: &Resolved) -> Option<Advert> {
    read_where(resolved, |ip| ip.is_loopback())
}

fn read_where(resolved: &Resolved, usable: impl Fn(&IpAddr) -> bool) -> Option<Advert> {
    if resolved.txt.get("proto").map(String::as_str) != Some("2") || resolved.port == 0 {
        return None;
    }
    let nonce = lower_hex::<ADVERT_NONCE_LEN>(resolved.txt.get("n")?)?;
    let all = lower_hex::<{ HINT_SLOTS * HINT_LEN }>(resolved.txt.get("h")?)?;
    let hints = <[[u8; HINT_LEN]; HINT_SLOTS]>::try_from(all.as_chunks::<HINT_LEN>().0).ok()?;
    let ip = resolved
        .addresses
        .iter()
        .filter(|ip| usable(ip))
        .min_by_key(|ip| match ip {
            IpAddr::V4(_) => 0,
            IpAddr::V6(_) => 1,
        })?;
    let pairing = resolved
        .txt
        .get("id")
        .and_then(|id| parse_device_id(id))
        .map(|device_id| PairingMode {
            device_id,
            // The character rule of names on the wire, so no control, bidi or zero-width
            // character reaches the UI.
            name: resolved
                .txt
                .get("name")
                .map(|n| {
                    n.chars()
                        .filter(|&c| wire::name_char_allowed(c))
                        .collect::<String>()
                })
                .filter(|n| !n.trim().is_empty() && n.len() <= 48)
                .unwrap_or_else(|| "TouchDeck".to_owned()),
            paired: resolved
                .txt
                .get("paired")
                .and_then(|p| p.parse().ok())
                .filter(|&p: &u8| usize::from(p) <= HINT_SLOTS),
        });
    Some(Advert {
        address: Address {
            host: ip.to_string(),
            port: resolved.port,
        },
        nonce,
        hints,
        pairing,
    })
}

/// Merges answers by nonce, which is one Device's record until it rotates, keeping the latest
/// for each, and at most [`MAX_FOUND`].
pub fn merge(found: impl IntoIterator<Item = Advert>) -> Vec<Advert> {
    let mut by_nonce = BTreeMap::new();
    for advert in found {
        if by_nonce.len() < MAX_FOUND || by_nonce.contains_key(&advert.nonce) {
            by_nonce.insert(advert.nonce, advert);
        }
    }
    by_nonce.into_values().collect()
}

/// Browses for `wait` (capped at [`MAX_BROWSE`]) and returns the Devices that answered.
pub fn browse(wait: Duration) -> Result<Vec<Advert>, String> {
    let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS couldn't start: {e}"))?;
    let events = daemon
        .browse(SERVICE)
        .map_err(|e| format!("mDNS couldn't browse: {e}"))?;
    let end = Instant::now() + wait.min(MAX_BROWSE);
    let mut found = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match events.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let resolved = Resolved {
                    addresses: info.addresses.iter().map(|a| a.to_ip_addr()).collect(),
                    port: info.port,
                    txt: info
                        .txt_properties
                        .iter()
                        .map(|p| (p.key().to_ascii_lowercase(), p.val_str().to_owned()))
                        .collect(),
                };
                found.extend(read(&resolved));
                if found.len() >= MAX_ANSWERS {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(SERVICE);
    let _ = daemon.shutdown();
    Ok(merge(found))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "5a5b5c5d5e5f60616263646566676869";
    const HINTS: &str = "0011223344556677707172737475767778797a7b7c7d7e7f8081828384858687";

    fn resolved(txt: &[(&str, &str)], addresses: &[&str]) -> Resolved {
        Resolved {
            addresses: addresses.iter().map(|a| a.parse().unwrap()).collect(),
            port: 47_820,
            txt: txt
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    const TXT: &[(&str, &str)] = &[("proto", "2"), ("n", NONCE), ("h", HINTS)];
    const PAIRING: &[(&str, &str)] = &[
        ("proto", "2"),
        ("n", NONCE),
        ("h", HINTS),
        ("id", "020000000001"),
        ("name", "Desk"),
        ("paired", "1"),
    ];

    #[test]
    fn a_record_outside_pairing_mode_names_nothing() {
        let found = read(&resolved(TXT, &["fe80::1", "2001:db8::5", "192.0.2.10"])).unwrap();
        assert_eq!(found.address.to_string(), "192.0.2.10:47820");
        assert_eq!(found.pairing, None);
        assert_eq!(
            found.hints[0],
            [0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]
        );
        assert_eq!(found.hints[3][7], 0x87);
        let v6 = read(&resolved(TXT, &["fe80::1", "2001:db8::5"])).unwrap();
        assert_eq!(v6.address.host, "2001:db8::5");
    }

    #[test]
    fn a_record_in_pairing_mode_names_the_device() {
        let found = read(&resolved(PAIRING, &["192.0.2.10"])).unwrap();
        let pairing = found.pairing.unwrap();
        assert_eq!(pairing.device_id, [2, 0, 0, 0, 0, 1]);
        assert_eq!(pairing.name, "Desk");
        assert_eq!(pairing.paired, Some(1));
    }

    #[test]
    fn loopback_link_local_and_odd_addresses_are_dropped() {
        for address in [
            "127.0.0.1",
            "::1",
            "169.254.1.2",
            "fe80::1",
            "0.0.0.0",
            "224.0.0.251",
            "ff02::fb",
            "255.255.255.255",
            "::ffff:127.0.0.1",
        ] {
            assert_eq!(read(&resolved(TXT, &[address])), None, "{address}");
        }
        let found = read(&resolved(TXT, &["127.0.0.1", "192.0.2.10"])).unwrap();
        assert_eq!(found.address.host, "192.0.2.10");
    }

    #[test]
    fn names_lose_every_character_the_wire_refuses() {
        let mut odd = resolved(PAIRING, &["192.0.2.10"]);
        odd.txt
            .insert("name".into(), "De\u{202e}sk\u{200b}\u{7}".into());
        assert_eq!(read(&odd).unwrap().pairing.unwrap().name, "Desk");
        odd.txt.insert("name".into(), "\u{200b}\u{3164}".into());
        assert_eq!(read(&odd).unwrap().pairing.unwrap().name, "TouchDeck");
        let mut many = resolved(PAIRING, &["192.0.2.10"]);
        many.txt.insert("paired".into(), "9".into());
        assert_eq!(read(&many).unwrap().pairing.unwrap().paired, None);
    }

    #[test]
    fn a_browse_returns_at_most_thirty_two_devices() {
        let found = (0..40u8).map(|i| {
            let mut answer = resolved(TXT, &["192.0.2.10"]);
            answer
                .txt
                .insert("n".into(), format!("{i:02x}{}", &NONCE[2..]));
            read(&answer).unwrap()
        });
        assert_eq!(merge(found).len(), MAX_FOUND);
    }

    #[test]
    fn malformed_records_and_other_protocols_are_dropped() {
        let cases: &[(&str, &str)] = &[
            ("proto", "1"),
            ("n", &NONCE[2..]),
            ("h", &HINTS[16..]),
            ("n", "5A5B5C5D5E5F60616263646566676869"),
            (
                "h",
                "zz11223344556677707172737475767778797a7b7c7d7e7f8081828384858687",
            ),
        ];
        for (key, value) in cases {
            let mut bad = resolved(TXT, &["192.0.2.10"]);
            bad.txt.insert(key.to_string(), value.to_string());
            assert_eq!(read(&bad), None, "{key}={value}");
        }
        // An id alone, as an old record carried, is no record.
        let old = resolved(&[("proto", "2"), ("id", "020000000001")], &["192.0.2.10"]);
        assert_eq!(read(&old), None);
        assert_eq!(read(&resolved(TXT, &[])), None);
        // A bad id is no pairing mode, but the hints still count.
        let mut bad_id = resolved(PAIRING, &["192.0.2.10"]);
        bad_id.txt.insert("id".into(), "zz".into());
        assert_eq!(read(&bad_id).unwrap().pairing, None);
        let mut nameless = resolved(PAIRING, &["192.0.2.10"]);
        nameless.txt.remove("name");
        assert_eq!(read(&nameless).unwrap().pairing.unwrap().name, "TouchDeck");
    }

    #[test]
    fn answers_merge_by_nonce() {
        let a = read(&resolved(TXT, &["192.0.2.10"])).unwrap();
        let b = read(&resolved(TXT, &["192.0.2.11"])).unwrap();
        let merged = merge([a, b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].address.host, "192.0.2.11");
    }

    #[test]
    fn a_record_matches_only_the_pairing_whose_hint_it_carries() {
        let ltk = [0xe0u8; 32];
        let key = wire::discovery_key(&ltk);
        let mut found = read(&resolved(TXT, &["192.0.2.10"])).unwrap();
        assert!(!found.matches(&key));
        found.hints[2] = wire::hint(&key, &found.nonce);
        assert!(found.matches(&key));
        assert!(!found.matches(&wire::discovery_key(&[0x99; 32])));
        // A new nonce makes the old hint useless.
        found.nonce[0] ^= 1;
        assert!(!found.matches(&key));
    }
}
