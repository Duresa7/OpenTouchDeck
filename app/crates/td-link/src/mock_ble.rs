//! The mock Device's Bluetooth keyboard: the radio build's `ble.*` messages as the link shows them. No radio is
//! involved; a test plays the host's part with the `MockDeck` hooks.
//!
//! It follows the Firmware (`firmware/components/td_ble/td_ble_core.c`): `ble.set true` answers `starting`, and the
//! host syncs before the next request unless a test holds it back. Bonds are listed once the host has synced, and a
//! host reset keeps listing them while it says `starting` again. Turning Bluetooth off clears a failed start.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::asset::refusal;
use crate::ble::{PAIRING_MS, normalize_address, valid_address};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MockBond {
    pub address: String,
    pub random: bool,
    pub connected: bool,
}

#[derive(Debug, Default)]
pub(crate) struct MockBle {
    /// `caps.ble_hid`: the most bonds. `None` is the normal build, which doesn't know `ble.*`.
    pub bonds_max: Option<u32>,
    /// The stored setting.
    pub enabled: bool,
    /// The stack didn't start (`failed`).
    pub failed: bool,
    /// The stack runs but the host hasn't synced yet (`starting`).
    pub starting: bool,
    /// The host has read the bond store since the stack started, so the bonds are listed.
    pub listed: bool,
    /// The host stays unsynced until released, as a slow start or a host reset.
    pub hold_start: bool,
    /// The next starts fail, as a controller that won't come up.
    pub start_fails: bool,
    /// Writes to the Device's storage fail: the setting and bond deletions.
    pub storage_fails: bool,
    pub window_until: Option<Instant>,
    pub passkey: bool,
    /// The bond store, which outlives turning Bluetooth off.
    pub bonds: Vec<MockBond>,
}

impl MockBle {
    fn running(&self) -> bool {
        self.enabled && !self.failed
    }

    fn on(&self) -> bool {
        self.running() && !self.starting
    }

    /// The host syncs between requests unless a test holds it back.
    fn sync(&mut self) {
        if self.running() && self.starting && !self.hold_start {
            self.starting = false;
            self.listed = true;
        }
    }

    /// Closes a window whose time ran out, which also takes the passkey down.
    fn expire(&mut self) {
        if self
            .window_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.close();
        }
    }

    fn close(&mut self) {
        self.window_until = None;
        self.passkey = false;
    }

    fn stop(&mut self) {
        self.failed = false;
        self.starting = false;
        self.listed = false;
        self.close();
    }

    fn fields(&mut self) -> Map<String, Value> {
        self.expire();
        let on = self.on();
        let left = self
            .window_until
            .map_or(Duration::ZERO, |until| {
                until.saturating_duration_since(Instant::now())
            })
            .as_millis() as u64;
        let advertising = match (on, self.window_until.is_some()) {
            (false, _) => "off",
            (true, true) => "pairing",
            (true, false) if !self.bonds.is_empty() && !self.bonds.iter().any(|b| b.connected) => {
                "fast"
            }
            (true, false) => "off",
        };
        let state = if self.failed {
            "failed"
        } else if !self.enabled {
            "off"
        } else if self.starting {
            "starting"
        } else {
            "on"
        };
        let bonds: Vec<Value> = if self.running() && self.listed {
            self.bonds
                .iter()
                .map(
                    |b| json!({"address": b.address, "random": b.random, "connected": on && b.connected}),
                )
                .collect()
        } else {
            Vec::new()
        };
        let fields = json!({
            "enabled": self.enabled, "state": state, "pairing": self.window_until.is_some(),
            "pairing_ms": left, "passkey": self.passkey, "advertising": advertising, "bonds": bonds,
            "bonds_max": self.bonds_max.unwrap_or(0), "battery": 100,
        });
        fields.as_object().cloned().unwrap_or_default()
    }

    /// A refusal that carries the state, as every `ble.*` refusal but `args`, `unsupported` and `usb-only` does.
    fn refused(&mut self, reason: &str, error: &str) -> Map<String, Value> {
        let mut reply = refusal(reason, error);
        reply.extend(self.fields());
        reply
    }

    /// Why the window can't open or a bond can't be forgotten now.
    fn not_on(&mut self) -> Option<Map<String, Value>> {
        if !self.running() {
            return Some(self.refused("off", "Bluetooth is off"));
        }
        if self.starting {
            return Some(self.refused("starting", "Bluetooth is starting, try again"));
        }
        None
    }

    /// Answers one `ble.*` request. `Err` is the unknown-message error of a Device without the keyboard.
    pub fn handle(
        &mut self,
        m: &str,
        request: &Map<String, Value>,
        network: bool,
    ) -> Result<Map<String, Value>, String> {
        if self.bonds_max.is_none() {
            return Err("unknown message".into());
        }
        // The Device checks the Carrier before the handler runs: off USB, every `ble.*` is refused.
        if network {
            return Ok(refusal("usb-only", "Only over USB"));
        }
        let only = |keys: &[&str]| {
            request
                .keys()
                .all(|key| key == "m" || keys.contains(&key.as_str()))
        };
        let args = || Ok(refusal("args", "Invalid request fields"));
        self.sync();
        self.expire();
        match m {
            "ble.state" if only(&[]) => {}
            "ble.set" if only(&["enabled"]) => {
                let Some(enabled) = request.get("enabled").and_then(Value::as_bool) else {
                    return args();
                };
                if enabled != self.enabled && self.storage_fails {
                    return Ok(self.refused("storage", "Could not save the setting"));
                }
                if !enabled {
                    self.enabled = false;
                    self.stop();
                } else if !self.running() {
                    self.enabled = true;
                    if self.start_fails {
                        self.failed = true;
                        return Ok(self.refused("radio", "Bluetooth did not start"));
                    }
                    self.failed = false;
                    self.starting = true;
                    self.listed = false;
                }
            }
            "ble.pair" if only(&["open"]) => {
                let open = match request.get("open") {
                    None => true,
                    Some(Value::Bool(open)) => *open,
                    Some(_) => return args(),
                };
                if let Some(refused) = self.not_on() {
                    return Ok(refused);
                }
                if !open {
                    self.close();
                } else if self.window_until.is_none()
                    && self.bonds.len() >= self.bonds_max.unwrap_or(0) as usize
                {
                    return Ok(self.refused("full", "Forget a host first"));
                } else {
                    self.window_until =
                        Some(Instant::now() + Duration::from_millis(PAIRING_MS.into()));
                }
            }
            "ble.forget" if only(&["address"]) => {
                // The Firmware takes either case and lists addresses in lower case.
                let address = match request.get("address") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(address)) => match normalize_address(address) {
                        Some(address) => Some(address),
                        None => return args(),
                    },
                    Some(_) => return args(),
                };
                if let Some(refused) = self.not_on() {
                    return Ok(refused);
                }
                let matched = self
                    .bonds
                    .iter()
                    .any(|bond| address.as_ref().is_none_or(|a| *a == bond.address));
                if matched && self.storage_fails {
                    let mut reply = self.refused("storage", "Could not forget the host");
                    reply.insert("forgotten".into(), json!(0));
                    return Ok(reply);
                }
                let before = self.bonds.len();
                self.bonds
                    .retain(|bond| address.as_ref().is_some_and(|a| *a != bond.address));
                let forgotten = before - self.bonds.len();
                let mut reply = self.fields();
                reply.insert("forgotten".into(), json!(forgotten));
                return Ok(reply);
            }
            "ble.state" | "ble.set" | "ble.pair" | "ble.forget" => return args(),
            // A radio build answers a `ble.*` message it doesn't know with `unsupported`, and no state.
            _ => return Ok(refusal("unsupported", "Unknown Bluetooth message")),
        }
        Ok(self.fields())
    }

    /// The host resets and starts again, as NimBLE does after a controller error: the window and the links drop,
    /// the bonds stay listed.
    pub fn host_reset(&mut self) {
        if self.running() {
            self.starting = true;
            self.close();
            for bond in &mut self.bonds {
                bond.connected = false;
            }
        }
    }

    /// A host types the passkey and bonds inside the window. The new bond closes the window.
    pub fn host_pairs(&mut self, address: &str, random: bool) -> bool {
        self.expire();
        if !self.on() || self.window_until.is_none() || !valid_address(address) {
            return false;
        }
        self.bonds.retain(|bond| bond.address != address);
        for bond in &mut self.bonds {
            bond.connected = false;
        }
        self.bonds.push(MockBond {
            address: address.into(),
            random,
            connected: true,
        });
        self.close();
        true
    }
}
