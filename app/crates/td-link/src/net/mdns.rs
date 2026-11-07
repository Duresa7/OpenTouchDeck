//! Finding Devices on the LAN with DNS-SD over multicast DNS (`_touchdeck._tcp`). This uses
//! `mdns-sd`, a pure-Rust querier, so it needs neither Bonjour nor Avahi and works next to an
//! OS responder that already owns port 5353. A browse runs only when asked and stops when it
//! returns. Where multicast is blocked it finds nothing, and the person types an address.
//!
//! Records are untrusted hints: a Pairing pins the Device id and every session checks it.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use super::{Address, DeviceId, SERVICE, parse_device_id};

/// The longest a browse may run.
pub const MAX_BROWSE: Duration = Duration::from_secs(10);

/// One Device that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub device_id: DeviceId,
    pub name: String,
    pub address: Address,
    /// Stored Pairings on the Device, when it said.
    pub paired: Option<u8>,
}

/// What a resolved DNS-SD instance carried, before any of it is trusted.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub txt: BTreeMap<String, String>,
}

/// Reads one resolved instance. Anything that isn't net-v2 with a valid id and a usable
/// address is dropped. IPv4 is preferred, then a global IPv6 address, then a link-local one.
pub fn read(resolved: &Resolved) -> Option<Discovered> {
    if resolved.txt.get("proto").map(String::as_str) != Some("2") || resolved.port == 0 {
        return None;
    }
    let device_id = parse_device_id(resolved.txt.get("id")?)?;
    let ip = resolved
        .addresses
        .iter()
        .filter(|ip| !ip.is_unspecified() && !ip.is_multicast())
        .min_by_key(|ip| match ip {
            IpAddr::V4(_) => 0,
            IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) != 0xfe80 => 1,
            IpAddr::V6(_) => 2,
        })?;
    let name = resolved
        .txt
        .get("name")
        .map(|n| n.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|n| !n.trim().is_empty() && n.len() <= 48)
        .unwrap_or_else(|| "TouchDeck".to_owned());
    let paired = resolved.txt.get("paired").and_then(|p| p.parse().ok());
    Some(Discovered {
        device_id,
        name,
        address: Address {
            host: ip.to_string(),
            port: resolved.port,
        },
        paired,
    })
}

/// Merges answers by Device id, keeping the latest for each.
pub fn merge(found: impl IntoIterator<Item = Discovered>) -> Vec<Discovered> {
    let mut by_id = BTreeMap::new();
    for device in found {
        by_id.insert(device.device_id, device);
    }
    by_id.into_values().collect()
}

/// Browses for `wait` (capped at [`MAX_BROWSE`]) and returns the Devices that answered.
pub fn browse(wait: Duration) -> Result<Vec<Discovered>, String> {
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

    const TXT: &[(&str, &str)] = &[
        ("id", "020000000001"),
        ("proto", "2"),
        ("name", "Desk"),
        ("paired", "1"),
    ];

    #[test]
    fn a_net_v2_record_reads_as_a_device() {
        let found = read(&resolved(TXT, &["fe80::1", "2001:db8::5", "192.0.2.10"])).unwrap();
        assert_eq!(found.device_id, [2, 0, 0, 0, 0, 1]);
        assert_eq!(found.name, "Desk");
        assert_eq!(found.address.to_string(), "192.0.2.10:47820");
        assert_eq!(found.paired, Some(1));
        let v6 = read(&resolved(TXT, &["fe80::1", "2001:db8::5"])).unwrap();
        assert_eq!(v6.address.host, "2001:db8::5");
    }

    #[test]
    fn other_protocols_and_bad_ids_are_dropped() {
        let mut v1 = resolved(TXT, &["192.0.2.10"]);
        v1.txt.insert("proto".into(), "1".into());
        assert_eq!(read(&v1), None);
        let mut bad = resolved(TXT, &["192.0.2.10"]);
        bad.txt.insert("id".into(), "zz".into());
        assert_eq!(read(&bad), None);
        assert_eq!(read(&resolved(TXT, &[])), None);
        let mut nameless = resolved(TXT, &["192.0.2.10"]);
        nameless.txt.remove("name");
        assert_eq!(read(&nameless).unwrap().name, "TouchDeck");
    }

    #[test]
    fn answers_merge_by_device_id() {
        let a = read(&resolved(TXT, &["192.0.2.10"])).unwrap();
        let b = read(&resolved(TXT, &["192.0.2.11"])).unwrap();
        let merged = merge([a, b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].address.host, "192.0.2.11");
    }
}
