//! Compact legacy BLE service data for net-v2. Radio integration supplies the
//! service UUID and must process all four slots, including changed scan responses.
use super::{DeviceId, wire};

/// A discovery candidate, never proof of identity or permission to replace an endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Advert {
    Private { nonce: [u8; 6], hint: [u8; 6] },
    Pairing { device_id: DeviceId, paired: u8 },
}

/// Parse service data after the net-v2 UUID; reject legacy stable-id payloads.
pub fn read(data: &[u8]) -> Option<Advert> {
    let data: &[u8; 13] = data.try_into().ok()?;
    match data[0] {
        2 => Some(Advert::Private {
            nonce: data[1..7].try_into().ok()?,
            hint: data[7..13].try_into().ok()?,
        }),
        0x82 if data[7] <= 4 => Some(Advert::Pairing {
            device_id: data[1..7].try_into().ok()?,
            paired: data[7],
        }),
        _ => None,
    }
}

impl Advert {
    /// Compare one slot against a Pairing's derived discovery key in constant time.
    /// Callers must check all stored keys and authenticate the resulting candidate.
    pub fn matches(&self, discovery_key: &[u8; 32]) -> bool {
        match self {
            Self::Private { nonce, hint } => {
                let mac = wire::hmac(discovery_key, &[b"ble-hint", nonce]);
                wire::same(&mac[..6], hint)
            }
            Self::Pairing { .. } => false,
        }
    }
}
