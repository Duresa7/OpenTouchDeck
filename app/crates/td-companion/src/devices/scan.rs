//! The scan thread. It wakes on a USB change, a kick or every second, reconciles the port
//! list with what it knows, and probes ports that are due. A port that didn't answer as a
//! Device is probed again with backoff.
use super::{
    BoardProbe, Capabilities, Carrier, DeviceAdded, DeviceId, DeviceSummary, Entry, FirmwareInfo,
    Inner, LinkState, LostReason, NewBoard, NewBoardAdded, NewBoardRemoved, PortInfo, PortRecord,
    PortSource, PortStatus, Registry, SetupState, actor,
};
use std::{
    io,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use td_link::{Identity, LinkError, Session, SessionConfig, discover::HELLO_TIMEOUT};

/// The fallback poll when no USB change arrives.
const POLL: Duration = Duration::from_secs(1);
/// The first retry after a probe that found no Device. Each retry doubles it up to `MAX_RETRY`.
pub(super) const FIRST_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(10);
/// A board that just appeared may still be booting. Until this passes, silence only means
/// "try again", so a Device being plugged in isn't shown as unresponsive or as a New board.
const BOOT_GRACE: Duration = Duration::from_secs(3);
const STEADY_LINK: Duration = Duration::from_secs(30);

pub(super) fn run(inner: Arc<Inner>) {
    let source = inner.config().source.clone();
    let wake = inner.clone();
    source.watch(Arc::new(move || wake.wake.kick()));
    while !inner.stop.load(Ordering::SeqCst) {
        let due = inner.reconcile(source.list());
        for (port, id) in due {
            if inner.stop.load(Ordering::SeqCst) {
                break;
            }
            let probed = probe(&*source, &port);
            inner.apply(&port, &id, probed);
        }
        let next = inner.next_due().min(POLL);
        inner.wake.wait(next);
    }
    // Dropping the handles ends every actor, which closes its session and port.
    let mut reg = inner.reg.lock();
    let actors: Vec<_> = reg
        .devices
        .values_mut()
        .filter_map(|e| e.actor.take())
        .collect();
    drop(reg);
    drop(actors);
}

/// A Device that answered, with the session that becomes its link.
pub(super) struct Found {
    pub session: Session,
    pub incoming: td_link::IncomingReceiver,
    pub identity: Identity,
    pub hello: td_core::Hello,
}
pub(super) enum Probed {
    Device(Box<Found>),
    Silent,
    Busy(String),
    OtherFirmware {
        product: String,
        detail: String,
    },
    /// The port couldn't be opened for another reason, such as leaving as it was opened.
    Unavailable,
}

/// Opens the port and says `hello` with a 500 ms timeout. A Device keeps the session.
fn probe(source: &dyn PortSource, port: &str) -> Probed {
    let transport = match source.open(port) {
        Ok(transport) => transport,
        Err(error)
            if error.kind() == io::ErrorKind::PermissionDenied
                || error.to_string().to_lowercase().contains("busy") =>
        {
            return Probed::Busy(error.to_string());
        }
        Err(_) => return Probed::Unavailable,
    };
    let config = SessionConfig {
        close_on_restart: true,
        close_on_drop: true,
        ..SessionConfig::default()
    };
    let Ok((session, incoming)) = Session::open(transport, config) else {
        return Probed::Unavailable;
    };
    let hello = match session.hello(HELLO_TIMEOUT) {
        Ok(hello) => hello,
        Err(LinkError::Timeout(_)) => return Probed::Silent,
        Err(error) => {
            return Probed::OtherFirmware {
                product: String::new(),
                detail: error.to_string(),
            };
        }
    };
    if !hello.is_deck() {
        return Probed::OtherFirmware {
            detail: format!("product {:?} protocol {}", hello.product, hello.proto),
            product: hello.product,
        };
    }
    let identity = match session.identity(HELLO_TIMEOUT) {
        Ok(identity) => identity,
        Err(LinkError::Timeout(_)) => return Probed::Silent,
        // A v2 reply that fails its checks falls back to v1, which `identity` re-selected.
        Err(error) => {
            eprintln!("Companion: {port} gave an invalid v2 hello ({error}); using protocol 1.");
            Identity {
                id: None,
                name: None,
                version: hello.version.clone(),
                capabilities: None,
                protocol: 1,
            }
        }
    };
    Probed::Device(Box::new(Found {
        session,
        incoming,
        identity,
        hello,
    }))
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(|c| c.to_lowercase())
        .take(32)
        .collect()
}
/// The id for a port: its serial cleaned, or `port` and the cleaned port name when the driver
/// reports none. If a port already holds that id, the newcomer gets a suffixed one.
fn assign_id(reg: &Registry, info: &PortInfo) -> (DeviceId, bool) {
    let serial = info
        .usb_serial
        .as_deref()
        .map(clean)
        .filter(|s| !s.is_empty());
    let base = serial.clone().unwrap_or_else(|| {
        format!("port{}", clean(&info.port))
            .chars()
            .take(32)
            .collect()
    });
    if reg.ports.values().any(|r| r.id.as_str() == base) {
        (suffixed_id(&base, &info.port), false)
    } else {
        (
            DeviceId::parse(&base).expect("a cleaned id is valid"),
            serial.is_some(),
        )
    }
}
/// The first 27 characters, `x`, and 4 hex digits of the port name's SHA-256. It stays the
/// same while the Device stays on that port.
pub(super) fn suffixed_id(base: &str, port: &str) -> DeviceId {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(port.as_bytes());
    let head: String = base.chars().take(27).collect();
    DeviceId::parse(&format!("{head}x{:02x}{:02x}", hash[0], hash[1]))
        .expect("a suffixed id is valid")
}
fn lost_reason(reason: &str) -> LostReason {
    if reason.contains("ping") {
        LostReason::Timeout
    } else if reason.contains("failed") {
        LostReason::IoError
    } else {
        LostReason::Closed
    }
}

impl Inner {
    /// Brings the registry in line with the listed ports and returns the ports to probe now.
    pub(super) fn reconcile(&self, listed: Vec<PortInfo>) -> Vec<(String, DeviceId)> {
        let now = Instant::now();
        let mut reg = self.reg.lock();
        let gone: Vec<String> = reg
            .ports
            .values()
            .filter(|r| !listed.contains(&r.info) && !reg.updating.contains(&r.id))
            .map(|r| r.info.port.clone())
            .collect();
        for port in gone {
            let record = reg.ports.remove(&port).expect("listed above");
            self.unplugged(&mut reg, &record.id);
        }
        for info in listed {
            if reg.ports.contains_key(&info.port) {
                continue;
            }
            let (id, stable) = assign_id(&reg, &info);
            if reg.devices.get(&id).is_some_and(|e| e.actor.is_none()) {
                let link = LinkState::Connecting {
                    port: info.port.clone(),
                };
                self.set_link(&mut reg, &id, link);
            }
            reg.ports.insert(
                info.port.clone(),
                PortRecord {
                    info,
                    id,
                    stable,
                    status: PortStatus::Pending,
                    probed_ms: None,
                    appeared: now,
                    due: now,
                    backoff: FIRST_RETRY,
                },
            );
        }
        reg.ports
            .values()
            .filter(|r| r.due <= now && wants_probe(&reg, &r.id))
            .map(|r| (r.info.port.clone(), r.id.clone()))
            .collect()
    }
    /// How long until the next probe is due.
    pub(super) fn next_due(&self) -> Duration {
        let reg = self.reg.lock();
        let now = Instant::now();
        reg.ports
            .values()
            .filter(|r| wants_probe(&reg, &r.id))
            .map(|r| r.due.saturating_duration_since(now))
            .min()
            .unwrap_or(Duration::MAX)
    }
    /// A port left. A New board goes; a Device stays known.
    fn unplugged(&self, reg: &mut Registry, id: &DeviceId) {
        if reg.updating.contains(id) {
            return;
        }
        if reg.boards.remove(id).is_some() {
            self.events.publish(&NewBoardRemoved { id: id.clone() });
        }
        let Some(entry) = reg.devices.get_mut(id) else {
            return;
        };
        // A Wi-Fi link isn't the port's; it keeps going.
        if entry
            .actor
            .as_ref()
            .is_some_and(|a| a.carrier == Carrier::Wifi)
        {
            return;
        }
        let linked = entry.actor.take().is_some();
        entry.auto = true;
        let link = if linked || matches!(entry.summary.link, LinkState::Lost { .. }) {
            LinkState::Lost {
                reason: LostReason::Unplugged,
                detail: "The Device was unplugged.".into(),
                at_ms: crate::now_ms(),
            }
        } else {
            LinkState::Offline
        };
        self.set_link(reg, id, link);
        // A paired Device falls back to Wi-Fi.
        self.wifi_due_now(reg, id);
    }
    /// Records a probe's result.
    pub(super) fn apply(self: &Arc<Self>, port: &str, id: &DeviceId, probed: Probed) {
        let mut reg = self.reg.lock();
        // The port may have left, or the user disconnected, while the probe ran.
        if !reg.ports.get(port).is_some_and(|r| &r.id == id) || !wants_probe(&reg, id) {
            drop(reg);
            drop(probed);
            return;
        }
        let record = reg.ports.get_mut(port).expect("checked above");
        record.probed_ms = Some(crate::now_ms());
        let found = match probed {
            Probed::Device(found) => *found,
            Probed::Unavailable => {
                retry(record);
                return;
            }
            other => {
                // A known Device's port may still be closing after its own link let go.
                let known = reg.devices.contains_key(id);
                let record = reg.ports.get_mut(port).expect("checked above");
                let waiting = match other {
                    Probed::Silent => true,
                    Probed::Busy(_) => known,
                    _ => false,
                };
                let booting = waiting && record.appeared.elapsed() < BOOT_GRACE;
                let (status, board) = match other {
                    Probed::Busy(detail) => (
                        PortStatus::Busy {
                            detail: detail.clone(),
                        },
                        BoardProbe::Busy { detail },
                    ),
                    Probed::OtherFirmware { product, detail } => (
                        PortStatus::OtherFirmware {
                            product: product.clone(),
                            detail: detail.clone(),
                        },
                        BoardProbe::OtherFirmware { product, detail },
                    ),
                    _ => (PortStatus::Silent, BoardProbe::Silent),
                };
                record.status = status;
                retry(record);
                if !booting {
                    self.classify(&mut reg, port, id, board);
                }
                return;
            }
        };
        record.status = PortStatus::Device {
            protocol: found.identity.protocol,
            version: found.identity.version.clone(),
        };
        record.backoff = FIRST_RETRY;
        let stable = record.stable;
        if let Some(usb) = record.info.usb_serial.as_deref()
            && found.identity.id.is_some()
            && !found.identity.matches_usb_serial(usb)
        {
            eprintln!(
                "Companion: {port} reports Device id {:?} but its USB serial is {usb}; using the USB serial.",
                found.identity.id
            );
        }
        // USB wins: a Wi-Fi link gives way.
        let old = self.link_up(&mut reg, id, port.into(), Carrier::Usb, stable, found);
        drop(reg);
        drop(old);
    }
    /// Starts the actor for a Device that answered on `carrier`, and records it as connected
    /// through `port`. Returns the actor it replaced, to drop after the lock.
    pub(super) fn link_up(
        self: &Arc<Self>,
        reg: &mut Registry,
        id: &DeviceId,
        port: String,
        carrier: Carrier,
        stable: bool,
        found: Found,
    ) -> Option<actor::Handle> {
        let Found {
            session,
            incoming,
            identity,
            hello,
        } = found;
        if reg.boards.remove(id).is_some() {
            self.events.publish(&NewBoardRemoved { id: id.clone() });
        }
        reg.generation += 1;
        let generation = reg.generation;
        let (mirror, key_mirror) = actor::mirror(&self.config().input, &hello);
        let session_id = crate::now_ms();
        let handle = actor::spawn(
            self.clone(),
            id.clone(),
            (generation, carrier),
            session_id,
            session,
            incoming,
            mirror,
        );
        let link = LinkState::Connected {
            port,
            since_ms: session_id,
            protocol: identity.protocol,
        };
        let firmware = FirmwareInfo {
            version: identity.version.clone(),
            protocol: identity.protocol,
            build: hello.elf_sha256.clone(),
            recovery: hello.recovery,
            update_available: None,
        };
        let diagnostics = super::DeviceDiagnostics {
            product: hello.product.clone(),
            running_from: hello.running.clone(),
            sinks: hello.sinks.clone(),
            current_layout: hello.layout.clone(),
        };
        let capabilities = identity.capabilities.as_ref().map(Capabilities::from);
        match reg.devices.get_mut(id) {
            Some(entry) => {
                let old = entry.actor.replace(handle);
                let saved_name = entry.summary.name.clone();
                self.update(reg, id, |d| {
                    d.name = identity.name.clone().or(saved_name);
                    d.link = link;
                    d.carrier = Some(carrier);
                    d.firmware = Some(firmware);
                    d.diagnostics = Some(diagnostics);
                    d.capabilities = capabilities;
                    d.key_mirror = key_mirror;
                    d.stable_id = stable;
                });
                if identity.name.is_some() {
                    self.save(reg);
                }
                old
            }
            None => {
                let summary = DeviceSummary {
                    id: id.clone(),
                    rev: 1,
                    name: identity.name.clone(),
                    setup: if reg.installed.remove(id) {
                        SetupState::NeedsSetup
                    } else {
                        SetupState::Ready
                    },
                    link,
                    carrier: Some(carrier),
                    firmware: Some(firmware),
                    diagnostics: Some(diagnostics),
                    capabilities,
                    orientation: None,
                    layout: None,
                    control_layout: super::ControlLayout::default(),
                    key_mirror,
                    stable_id: stable,
                };
                self.events.publish(&DeviceAdded(summary.clone()));
                reg.devices.insert(
                    id.clone(),
                    Entry {
                        summary,
                        actor: Some(handle),
                        auto: true,
                        quick_drops: 0,
                        shown: None,
                        reported_page: None,
                    },
                );
                self.save(reg);
                None
            }
        }
    }
    /// A port that didn't answer as a Device. A known Device stays a Device; an unknown
    /// serial is a New board.
    fn classify(&self, reg: &mut Registry, port: &str, id: &DeviceId, board: BoardProbe) {
        if let Some(entry) = reg.devices.get(id) {
            // A lost link keeps retrying as `lost`; it was a Device a moment ago. A Wi-Fi
            // link keeps its state.
            if matches!(entry.summary.link, LinkState::Lost { .. }) || entry.actor.is_some() {
                return;
            }
            let link = match board {
                BoardProbe::Busy { detail } => LinkState::Busy {
                    port: port.into(),
                    detail,
                },
                _ => LinkState::Unresponsive { port: port.into() },
            };
            self.set_link(reg, id, link);
            return;
        }
        if reg.boards.get(id).is_some_and(|b| b.probe == board) {
            return;
        }
        let board = NewBoard {
            id: id.clone(),
            port: port.into(),
            probe: board,
            seen_ms: crate::now_ms(),
        };
        reg.boards.insert(id.clone(), board.clone());
        self.events.publish(&NewBoardAdded(board));
    }
    /// The actor's session closed on its own.
    pub(super) fn closed(&self, id: &DeviceId, generation: u64, reason: &str) {
        let mut reg = self.reg.lock();
        if reg.updating.contains(id) {
            return;
        }
        let Some(entry) = reg.devices.get_mut(id) else {
            return;
        };
        if entry.actor.as_ref().map(|a| a.connection) != Some(generation) {
            return;
        }
        let actor = entry.actor.take();
        entry.quick_drops = match actor.as_ref() {
            Some(actor) if actor.opened.elapsed() < STEADY_LINK => {
                entry.quick_drops.saturating_add(1)
            }
            _ => 0,
        };
        let wait = relink_wait(entry.quick_drops);
        let link = LinkState::Lost {
            reason: lost_reason(reason),
            detail: reason.into(),
            at_ms: crate::now_ms(),
        };
        self.set_link(&mut reg, id, link);
        if actor.as_ref().is_some_and(|a| a.carrier == Carrier::Wifi) {
            super::wifi::relinked(&mut reg, id, wait);
            self.wifi.wake.kick();
        } else if let Some(record) = reg.ports.values_mut().find(|r| &r.id == id) {
            // The Device may be restarting, so silence gets the boot grace again.
            let now = Instant::now();
            record.status = PortStatus::Pending;
            record.appeared = now;
            record.due = now + wait;
            record.backoff = FIRST_RETRY;
        }
        drop(reg);
        drop(actor);
        self.wake.kick();
    }
}
fn relink_wait(quick_drops: u32) -> Duration {
    match quick_drops {
        0 => Duration::ZERO,
        n => (FIRST_RETRY * 2_u32.saturating_pow(n - 1)).min(MAX_RETRY),
    }
}
fn wants_probe(reg: &Registry, id: &DeviceId) -> bool {
    if reg.updating.contains(id) {
        return false;
    }
    // A Device linked over Wi-Fi is still probed on USB, which wins.
    reg.devices
        .get(id)
        .is_none_or(|e| e.auto && e.actor.as_ref().is_none_or(|a| a.carrier == Carrier::Wifi))
}
fn retry(record: &mut PortRecord) {
    record.due = Instant::now() + record.backoff;
    record.backoff = (record.backoff * 2).min(MAX_RETRY);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(reg: &mut Registry, port: &str, serial: Option<&str>) -> (DeviceId, bool) {
        let info = PortInfo {
            port: port.into(),
            usb_serial: serial.map(str::to_owned),
        };
        let (id, stable) = assign_id(reg, &info);
        let now = Instant::now();
        reg.ports.insert(
            port.into(),
            PortRecord {
                info,
                id: id.clone(),
                stable,
                status: PortStatus::Pending,
                probed_ms: None,
                appeared: now,
                due: now,
                backoff: FIRST_RETRY,
            },
        );
        (id, stable)
    }

    #[test]
    fn serials_that_clean_alike_get_distinct_ids_and_the_holder_keeps_its_own() {
        let mut reg = Registry::default();
        assert_eq!(
            record(&mut reg, "COM5", Some("AB:12")),
            (DeviceId::parse("ab12").unwrap(), true)
        );
        let (second, stable) = record(&mut reg, "COM3", Some("ab12"));
        assert_eq!(
            (second.clone(), stable),
            (suffixed_id("ab12", "COM3"), false)
        );
        assert_ne!(second, "ab12");
    }

    #[test]
    fn ids_stay_within_the_contract_pattern() {
        let mut reg = Registry::default();
        let long = "a".repeat(40);
        let (first, _) = record(&mut reg, "COM1", Some(&long));
        let (second, _) = record(&mut reg, "COM2", Some(&long));
        // No serial: `port` and the port name, with no hyphen.
        let (fallback, stable) = record(&mut reg, "/dev/ttyACM0", None);
        assert_eq!(fallback, "portdevttyacm0");
        assert!(!stable);
        for id in [first, second, fallback] {
            assert!(DeviceId::parse(id.as_str()).is_some(), "{id}");
            assert!(id.as_str().len() <= 32);
        }
    }

    #[test]
    fn close_reasons_map_to_lost_reasons() {
        assert_eq!(
            lost_reason("the deck stopped answering ping"),
            LostReason::Timeout
        );
        assert_eq!(lost_reason("read failed: gone"), LostReason::IoError);
        assert_eq!(
            lost_reason("heartbeat failed: write failed: x"),
            LostReason::IoError
        );
        assert_eq!(
            lost_reason("the deck closed the stream"),
            LostReason::Closed
        );
        assert_eq!(lost_reason(td_link::session::DROPPED), LostReason::Closed);
    }

    #[test]
    fn quick_drops_back_off_to_the_retry_cap() {
        let waits: Vec<u64> = (0..9).map(|n| relink_wait(n).as_millis() as u64).collect();
        assert_eq!(
            waits,
            [0, 250, 500, 1_000, 2_000, 4_000, 8_000, 10_000, 10_000]
        );
        assert_eq!(relink_wait(u32::MAX), MAX_RETRY);
    }
}
