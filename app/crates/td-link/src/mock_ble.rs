//! The mock Device's Bluetooth keyboard: the radio build's `ble.*` messages as the link shows them. No radio is
//! involved; a test plays the host's part with the `MockDeck` hooks.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::asset::refusal;
use crate::ble::{PAIRING_MS, valid_address};

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
    /// The next starts fail, as a controller that won't come up.
    pub start_fails: bool,
    pub window_until: Option<Instant>,
    pub passkey: bool,
    /// The bond store, which outlives turning Bluetooth off.
    pub bonds: Vec<MockBond>,
}

impl MockBle {
    fn on(&self) -> bool {
        self.enabled && !self.failed
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
        let state = match (self.enabled, self.failed) {
            (false, _) => "off",
            (true, true) => "failed",
            (true, false) => "on",
        };
        let bonds: Vec<Value> = if on {
            self.bonds
                .iter()
                .map(
                    |b| json!({"address": b.address, "random": b.random, "connected": b.connected}),
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
        self.expire();
        match m {
            "ble.state" if only(&[]) => {}
            "ble.set" if only(&["enabled"]) => {
                let Some(enabled) = request.get("enabled").and_then(Value::as_bool) else {
                    return args();
                };
                self.enabled = enabled;
                self.failed = enabled && self.start_fails;
                if !self.on() {
                    self.close();
                }
                if self.failed {
                    return Ok(self.refused("radio", "Bluetooth did not start"));
                }
            }
            "ble.pair" if only(&["open"]) => {
                let open = match request.get("open") {
                    None => true,
                    Some(Value::Bool(open)) => *open,
                    Some(_) => return args(),
                };
                if !self.on() {
                    return Ok(self.refused("off", "Bluetooth is off"));
                }
                if !open {
                    self.close();
                } else if self.bonds.len() >= self.bonds_max.unwrap_or(0) as usize {
                    return Ok(self.refused("full", "Forget a host first"));
                } else {
                    self.window_until =
                        Some(Instant::now() + Duration::from_millis(PAIRING_MS.into()));
                }
            }
            "ble.forget" if only(&["address"]) => {
                let address = match request.get("address") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(address)) if valid_address(address) => Some(address.clone()),
                    Some(_) => return args(),
                };
                if !self.on() {
                    return Ok(self.refused("off", "Bluetooth is off"));
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
            _ => return Err("unknown message".into()),
        }
        Ok(self.fields())
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
