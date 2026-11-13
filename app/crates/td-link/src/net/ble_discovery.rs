//! Compact legacy BLE service data for net-v2 discovery (net-v2 "Discovery privacy").
//!
//! Outside pairing mode a Device's scan response carries `0x02 || nonce[6] || hint[6]` and
//! cycles one hint slot per second; in pairing mode it carries
//! `0x82 || device_id[6] || paired || padding[5]`. The radio integration (#278) supplies the
//! service UUID, scans for long enough to see all four slots, and feeds every service-data
//! payload it sees, including changed ones at a known address, to [`recognise`].
use std::collections::BTreeSet;

use super::{DeviceId, wire};

/// Service data length after the 128-bit service UUID.
pub const LEN: usize = 13;
/// Marker of a private (hint) payload.
pub const PRIVATE: u8 = 0x02;
/// Marker of a pairing-mode payload.
pub const PAIRING: u8 = 0x82;
/// Hint slots a Device cycles through, whatever its Pairing count.
pub const SLOTS: usize = 4;
pub const NONCE_LEN: usize = 6;
pub const HINT_LEN: usize = 6;

/// A discovery candidate, never proof of identity or permission to replace an endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Advert {
    Private {
        nonce: [u8; NONCE_LEN],
        hint: [u8; HINT_LEN],
    },
    Pairing {
        device_id: DeviceId,
        paired: u8,
    },
}

/// The BLE hint: the first 6 bytes of `MAC(K_disc, "ble-hint" || nonce)`.
pub fn hint(discovery_key: &[u8; 32], nonce: &[u8; NONCE_LEN]) -> [u8; HINT_LEN] {
    let mac = wire::hmac(discovery_key, &[b"ble-hint", nonce]);
    mac[..HINT_LEN].try_into().expect("6 bytes")
}

/// Parse service data after the net-v2 UUID. Every other length or marker, and a Pairing
/// count above four, is `None`; the old stable-id format is never accepted.
pub fn read(data: &[u8]) -> Option<Advert> {
    let data: &[u8; LEN] = data.try_into().ok()?;
    match data[0] {
        PRIVATE => Some(Advert::Private {
            nonce: data[1..7].try_into().ok()?,
            hint: data[7..13].try_into().ok()?,
        }),
        PAIRING if usize::from(data[7]) <= SLOTS => Some(Advert::Pairing {
            device_id: data[1..7].try_into().ok()?,
            paired: data[7],
        }),
        _ => None,
    }
}

impl Advert {
    /// Whether this payload carries the hint of the Pairing with `discovery_key`, compared in
    /// constant time. A pairing-mode payload never matches: it names a Device, not a Pairing.
    pub fn matches(&self, discovery_key: &[u8; 32]) -> bool {
        match self {
            Self::Private { nonce, hint: seen } => wire::same(&hint(discovery_key, nonce), seen),
            Self::Pairing { .. } => false,
        }
    }
}

/// The Pairings whose Device sent one of `seen`, the raw service-data payloads of one scan.
///
/// Every payload is checked against every stored discovery key, without stopping at the first
/// match, and the result is deduplicated by Pairing. Payloads that fail to parse or match no key
/// are someone else's Device and are dropped. A match only says where to try a session: the
/// session still proves the LTK and pins `device_id`.
pub fn recognise<'a, K: Clone + Ord>(
    seen: impl IntoIterator<Item = &'a [u8]>,
    keys: &[(K, [u8; 32])],
) -> BTreeSet<K> {
    let mut found = BTreeSet::new();
    for advert in seen.into_iter().filter_map(read) {
        for (pairing, key) in keys {
            if advert.matches(key) {
                found.insert(pairing.clone());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private(key: Option<&[u8; 32]>, nonce: [u8; NONCE_LEN], filler: [u8; HINT_LEN]) -> Vec<u8> {
        let hint = key.map_or(filler, |key| hint(key, &nonce));
        [&[PRIVATE][..], &nonce, &hint].concat()
    }

    #[test]
    fn a_scan_recognises_only_this_companions_pairings() {
        let mine = wire::discovery_key(&[0x11; 32]);
        let also_mine = wire::discovery_key(&[0x22; 32]);
        let someone_elses = wire::discovery_key(&[0x33; 32]);
        let keys = [("mine", mine), ("also mine", also_mine)];
        // One Device paired here cycling its slots, and a Device paired elsewhere.
        let scan = [
            private(Some(&mine), [1; 6], [0xa0; 6]),
            private(None, [1; 6], [0xa1; 6]),
            private(Some(&someone_elses), [2; 6], [0xa2; 6]),
            private(None, [2; 6], [0xa3; 6]),
        ];
        let found = recognise(scan.iter().map(Vec::as_slice), &keys);
        assert_eq!(found, BTreeSet::from(["mine"]));
        // An unpaired Device sends only fillers and is recognised by nobody.
        let unpaired = [private(None, [3; 6], [0xa4; 6])];
        assert!(recognise(unpaired.iter().map(Vec::as_slice), &keys).is_empty());
    }

    #[test]
    fn a_device_stays_recognisable_across_rotations_and_is_deduplicated() {
        let key = wire::discovery_key(&[0x44; 32]);
        let keys = [(7u8, key)];
        let scan: Vec<Vec<u8>> = (0..4u8)
            .map(|epoch| private(Some(&key), [epoch; 6], [0xb0; 6]))
            .collect();
        assert_eq!(
            recognise(scan.iter().map(Vec::as_slice), &keys),
            BTreeSet::from([7])
        );
        // A hint replayed under another nonce matches nothing.
        let mut replayed = scan[0].clone();
        replayed[1] ^= 1;
        assert!(recognise([replayed.as_slice()], &keys).is_empty());
    }

    #[test]
    fn pairing_mode_and_malformed_payloads_match_no_pairing() {
        let key = wire::discovery_key(&[0x55; 32]);
        let pairing = [&[PAIRING][..], &[2, 0, 0, 0, 0, 1], &[1], &[9; 5]].concat();
        assert_eq!(
            read(&pairing),
            Some(Advert::Pairing {
                device_id: [2, 0, 0, 0, 0, 1],
                paired: 1
            })
        );
        let mut legacy = vec![2, 2, 0, 0, 0, 0, 1, 0];
        assert_eq!(read(&legacy), None, "the old proto/id/paired payload");
        legacy.resize(14, 0);
        assert!(recognise([pairing.as_slice(), &legacy], &[((), key)]).is_empty());
    }
}
