use crate::bundle::{Bundle, sha256};
use crate::recover::Recovery;
use crate::{Incoming, LinkError, Session, SessionConfig, XferFrame};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use td_core::frame::CHUNK_DATA_MAX;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum UpdateState {
    Checking,
    Sending { pct: u8 },
    Rebooting,
    Confirming,
    Done { message: String },
    Failed { reason: String },
    Recovering { pct: u8 },
}
pub trait Connector {
    fn connect(&mut self) -> Result<(Session, crate::IncomingReceiver), String>;
}
pub struct PortConnector<'a>(pub &'a str);
impl Connector for PortConnector<'_> {
    fn connect(&mut self) -> Result<(Session, crate::IncomingReceiver), String> {
        Session::open(
            Box::new(crate::Serial::open(self.0).map_err(|e| e.to_string())?),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .map_err(|e| e.to_string())
    }
}
impl Connector for crate::MockDeck {
    fn connect(&mut self) -> Result<(Session, crate::IncomingReceiver), String> {
        Session::open(
            Box::new(self.try_connect().map_err(|e| e.to_string())?),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy)]
pub struct Deadlines {
    /// How long the Device may keep answering as its old build after the restart was sent before
    /// the update stops waiting for it to go away.
    pub gone: Duration,
    pub reconnect: Duration,
    pub confirm: Duration,
    pub tick: Duration,
}
impl Default for Deadlines {
    fn default() -> Self {
        Self {
            gone: Duration::from_secs(3),
            reconnect: Duration::from_secs(15),
            confirm: Duration::from_secs(30),
            tick: Duration::from_millis(250),
        }
    }
}
const REQUEST: Duration = Duration::from_secs(2);
fn state(session: &Session, incoming: &crate::IncomingReceiver) -> Result<Value, String> {
    session
        .send_line("update state")
        .map_err(|e| e.to_string())?;
    let end = Instant::now() + REQUEST;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Ok(Incoming::Line(line)) = incoming.recv_timeout(left)
            && let Some(at) = line.find("@td ")
            && let Ok(value) = serde_json::from_str::<Value>(&line[at + 4..])
            && value["cmd"] == "update"
            && value["op"] == "state"
            && value["ok"] == true
        {
            return Ok(value);
        }
    }
    Err("no update state reply".into())
}
fn checked(session: &Session, message: Value, timeout: Duration) -> Result<Value, String> {
    let reply = session.call(&message, timeout).map_err(|e| e.to_string())?;
    if !reply.ok {
        return Err(reply.to_value().to_string());
    }
    Ok(reply.to_value())
}
/// How `update.begin` waits for the person at the Device on a network session (net-v2).
#[derive(Clone, Copy, Debug)]
pub struct Approval {
    /// The gap between repeats; net-v2 allows one every 2000 ms at most.
    pub repeat: Duration,
    /// How long to keep asking: the Device's prompt lasts 60000 ms.
    pub limit: Duration,
}

impl Default for Approval {
    fn default() -> Self {
        Self {
            repeat: Duration::from_secs(2),
            limit: Duration::from_secs(62),
        }
    }
}

/// Sends `update.begin` and returns its reply once a transfer opens. Over USB that's the
/// first reply. On a network session the Device answers `confirm` while it shows its Install
/// prompt: `prompted` runs once so the person can be told to look at the Device, and the
/// same request repeats until the Device opens the transfer or answers `declined`.
pub fn begin(
    session: &Session,
    bytes: usize,
    sha256: &str,
    approval: Approval,
    prompted: &mut dyn FnMut(),
) -> Result<Value, String> {
    let message = json!({"m":"update.begin","bytes":bytes,"sha256":sha256});
    let start = Instant::now();
    let mut asked_prompt = false;
    loop {
        let asked = Instant::now();
        let reply = session
            .call(&message, Duration::from_secs(15))
            .map_err(|e| e.to_string())?;
        if reply.ok {
            return Ok(reply.to_value());
        }
        match reply.fields.get("reason").and_then(Value::as_str) {
            Some("confirm") => {
                if !asked_prompt {
                    asked_prompt = true;
                    prompted();
                }
                if start.elapsed() >= approval.limit {
                    return Err("the update wasn't approved on the Device in time".into());
                }
                std::thread::sleep(approval.repeat.saturating_sub(asked.elapsed()));
            }
            Some("declined") => return Err("the update was declined on the Device".into()),
            _ => return Err(reply.to_value().to_string()),
        }
    }
}

/// How long an update waits for a transfer that is already running on the session.
const TRANSFER_WAIT: Duration = Duration::from_secs(10);

/// Takes the session's one transfer once it is free. The update shares the session with the rest of
/// the app, so a Layout read or an Asset upload that started just before it can still be running.
fn free_transfer(session: &Session) -> Result<crate::Transfer<'_>, String> {
    let end = Instant::now() + TRANSFER_WAIT;
    loop {
        match session.transfer() {
            Err(LinkError::TransferBusy) if Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result.map_err(|e| e.to_string()),
        }
    }
}

fn send(
    session: &Session,
    bundle: &Bundle,
    emit: &mut dyn FnMut(UpdateState),
) -> Result<String, String> {
    let transfer = free_transfer(session)?;
    let opened = begin(
        session,
        bundle.app().len(),
        &sha256(bundle.app()),
        Approval::default(),
        &mut || {},
    )?;
    let id = opened["xfer"]
        .as_u64()
        .and_then(|n| u8::try_from(n).ok())
        .filter(|n| *n != 0)
        .ok_or("bad update transfer id")?;
    let slot = opened["slot"]
        .as_str()
        .filter(|s| matches!(*s, "ota_0" | "ota_1"))
        .ok_or("bad update slot")?
        .to_owned();
    emit(UpdateState::Sending { pct: 0 });
    let chunks = bundle.app().chunks(CHUNK_DATA_MAX);
    let count = chunks.len();
    let mut shown = 0;
    for (idx, data) in chunks.enumerate() {
        let idx = u16::try_from(idx).map_err(|_| "image has too many chunks")?;
        let mut acked = false;
        for _ in 0..5 {
            transfer
                .send_chunk(id, idx, data)
                .map_err(|e| e.to_string())?;
            let end = Instant::now() + Duration::from_millis(200);
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                match transfer.frames.recv_timeout(left) {
                    Ok(XferFrame::Ack { xfer, next }) if xfer == id && next == idx + 1 => {
                        acked = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            if acked {
                break;
            }
        }
        if !acked {
            return Err(format!("chunk {idx} acknowledgement timeout"));
        }
        let pct = ((usize::from(idx) + 1) * 100 / count) as u8;
        if pct != shown {
            emit(UpdateState::Sending { pct });
            shown = pct;
        }
    }
    checked(
        session,
        json!({"m":"update.end","xfer":id}),
        Duration::from_secs(10),
    )?;
    Ok(slot)
}
fn confirm(
    connector: &mut dyn Connector,
    bundle: &Bundle,
    slot: Option<&str>,
    limits: Deadlines,
    emit: &mut dyn FnMut(UpdateState),
) -> Result<(), String> {
    let start = Instant::now();
    let mut gone = false;
    let mut confirming = None;
    loop {
        // Each attempt drops both serial handles before opening another connection.
        let reached = connector.connect().ok().and_then(|(session, incoming)| {
            let hello = session.hello(Duration::from_millis(500)).ok()?;
            hello.is_deck().then_some((session, incoming, hello))
        });
        if reached.is_none() {
            gone = true;
        }
        let ready = reached.filter(|(_, _, hello)| {
            let still_old_build = hello.elf_sha256 != bundle.manifest.elf_sha256;
            gone || !still_old_build || start.elapsed() >= limits.gone
        });
        if let Some((session, incoming, hello)) = ready {
            if confirming.is_none() {
                emit(UpdateState::Confirming);
                confirming = Some(Instant::now());
            }
            if let Ok(view) = state(&session, &incoming) {
                let wrong = hello.elf_sha256 != bundle.manifest.elf_sha256
                    || slot.is_some_and(|s| view["running"] != s);
                if wrong {
                    if slot.is_some_and(|s| view[s] == "ABORTED" || view[s] == "INVALID") {
                        return Err("rolled back".into());
                    }
                } else if view["confirmed"] == true {
                    return Ok(());
                }
            }
        }
        if let Some(at) = confirming {
            if at.elapsed() >= limits.confirm {
                return Err("confirm timeout".into());
            }
        } else if start.elapsed() >= limits.reconnect {
            return Err("port return timeout".into());
        }
        std::thread::sleep(limits.tick);
    }
}

/// Sends through the Device actor's existing session, then releases it before reconnecting.
pub fn run_session(
    session: &Session,
    connector: &mut dyn Connector,
    bundle: &Bundle,
    limits: Deadlines,
    emit: &mut dyn FnMut(UpdateState),
) -> Result<(), String> {
    emit(UpdateState::Checking);
    let hello = session
        .hello(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    if !hello.is_deck() {
        return Err("port is not a TouchDeck".into());
    }
    if hello.elf_sha256 == bundle.manifest.elf_sha256 {
        emit(UpdateState::Done {
            message: "Already on this build".into(),
        });
        return Ok(());
    }
    let slot = send(session, bundle, emit)?;
    session.close();
    emit(UpdateState::Rebooting);
    confirm(connector, bundle, Some(&slot), limits, emit)?;
    emit(UpdateState::Done {
        message: "Firmware updated".into(),
    });
    Ok(())
}

pub fn run(
    connector: &mut dyn Connector,
    recovery: Option<&mut dyn Recovery>,
    bundle: &Bundle,
    limits: Deadlines,
    emit: &mut dyn FnMut(UpdateState),
) -> Result<(), String> {
    emit(UpdateState::Checking);
    let result: Result<(), String> = (|| {
        let slot = if let Some(recovery) = recovery {
            emit(UpdateState::Recovering { pct: 0 });
            recovery.flash(bundle, emit)?;
            Some("ota_0".into())
        } else {
            let (session, _incoming) = connector.connect()?;
            let hello = session
                .hello(Duration::from_secs(10))
                .map_err(|e| e.to_string())?;
            if !hello.is_deck() {
                return Err("port is not a TouchDeck".into());
            }
            if hello.elf_sha256 == bundle.manifest.elf_sha256 {
                emit(UpdateState::Done {
                    message: "Already on this build".into(),
                });
                return Ok(());
            }
            Some(send(&session, bundle, emit)?)
        };
        emit(UpdateState::Rebooting);
        confirm(connector, bundle, slot.as_deref(), limits, emit)?;
        emit(UpdateState::Done {
            message: "Firmware updated".into(),
        });
        Ok(())
    })();
    if let Err(reason) = &result {
        emit(UpdateState::Failed {
            reason: reason.clone(),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle() -> Bundle {
        Bundle::parse(include_bytes!("../tests/fixtures/touchdeck-test.tdfw")).unwrap()
    }
    fn limits() -> Deadlines {
        Deadlines {
            gone: Duration::from_millis(10),
            reconnect: Duration::from_millis(50),
            confirm: Duration::from_millis(50),
            tick: Duration::from_millis(1),
        }
    }
    #[test]
    fn update_transfers_reboots_and_confirms() {
        let mut mock = crate::MockDeck::new();
        mock.lose_chunks([1]);
        let mut states = Vec::new();
        run(&mut mock, None, &bundle(), limits(), &mut |s| {
            states.push(s)
        })
        .unwrap();
        assert_eq!(states.first(), Some(&UpdateState::Checking));
        assert!(states.contains(&UpdateState::Sending { pct: 100 }));
        assert!(states.contains(&UpdateState::Rebooting));
        assert!(states.contains(&UpdateState::Confirming));
        assert!(matches!(states.last(), Some(UpdateState::Done { .. })));
        assert_eq!(mock.reboots(), 1);
        let received = mock.received();
        assert!(
            received
                .iter()
                .any(|r| r["m"] == "update.begin" && r["sha256"] == sha256(bundle().app()))
        );
        assert!(received.iter().any(|r| r["m"] == "update.end"));
        let mut again = Vec::new();
        run(&mut mock, None, &bundle(), limits(), &mut |s| again.push(s)).unwrap();
        assert_eq!(
            again,
            [
                UpdateState::Checking,
                UpdateState::Done {
                    message: "Already on this build".into()
                }
            ]
        );
        assert_eq!(mock.reboots(), 1);
    }
    #[test]
    fn an_update_waits_for_a_transfer_already_running_on_the_session() {
        let mut mock = crate::MockDeck::new();
        let (session, _incoming) = Connector::connect(&mut mock).unwrap();
        // Another part of the app, such as a Layout read, holds the session's one transfer.
        let reading = session.transfer().unwrap();
        std::thread::scope(|scope| {
            let mut connector = mock.clone();
            let session = &session;
            let update = scope.spawn(move || {
                run_session(session, &mut connector, &bundle(), limits(), &mut |_| {})
            });
            // The update asks for the transfer right after its hello.
            let end = Instant::now() + Duration::from_secs(5);
            while !mock.received().iter().any(|r| r["m"] == "hello") && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(100));
            assert!(!update.is_finished(), "the update waits for the transfer");
            drop(reading);
            update.join().unwrap().unwrap();
        });
        assert_eq!(mock.reboots(), 1);
    }
    #[test]
    fn missing_confirmation_fails() {
        let mut mock = crate::MockDeck::new();
        mock.update_mode("no-confirm");
        let mut states = Vec::new();
        assert_eq!(
            run(&mut mock, None, &bundle(), limits(), &mut |s| states
                .push(s)),
            Err("confirm timeout".into())
        );
        assert_eq!(
            states.last(),
            Some(&UpdateState::Failed {
                reason: "confirm timeout".into()
            })
        );
    }
    #[test]
    fn rollback_is_distinct_from_success() {
        let mut mock = crate::MockDeck::new();
        mock.update_mode("rollback");
        assert_eq!(
            run(&mut mock, None, &bundle(), limits(), &mut |_| {}),
            Err("rolled back".into())
        );
    }
    fn real_limits() -> Deadlines {
        Deadlines {
            gone: Duration::from_secs(2),
            reconnect: Duration::from_secs(5),
            confirm: Duration::from_secs(5),
            tick: Duration::from_millis(5),
        }
    }
    #[test]
    fn rebooting_holds_until_the_device_goes_away() {
        let mut mock = crate::MockDeck::new();
        let probe = mock.clone();
        mock.restart_after(Duration::from_millis(150), Duration::from_millis(100));
        let mut states = Vec::new();
        let mut early = false;
        run(&mut mock, None, &bundle(), real_limits(), &mut |s| {
            if s == UpdateState::Confirming {
                early = probe.port_drops_at().is_none_or(|at| Instant::now() < at);
            }
            states.push(s)
        })
        .unwrap();
        assert!(!early, "Confirming came before the port went away");
        let order: Vec<_> = states
            .iter()
            .filter(|s| {
                matches!(
                    s,
                    UpdateState::Rebooting | UpdateState::Confirming | UpdateState::Done { .. }
                )
            })
            .collect();
        assert!(matches!(
            order[..],
            [
                UpdateState::Rebooting,
                UpdateState::Confirming,
                UpdateState::Done { .. }
            ]
        ));
        assert_eq!(mock.reboots(), 1);
    }
    #[test]
    fn rebooting_gives_way_to_confirming_when_the_device_never_goes_away() {
        let mut mock = crate::MockDeck::new();
        mock.restart_after(Duration::from_secs(60), Duration::from_secs(1));
        let limits = Deadlines {
            gone: Duration::from_millis(100),
            confirm: Duration::from_millis(100),
            ..real_limits()
        };
        let mut rebooting = None;
        let mut waited = None;
        let mut states = Vec::new();
        let result = run(&mut mock, None, &bundle(), limits, &mut |s| {
            match s {
                UpdateState::Rebooting => rebooting = Some(Instant::now()),
                UpdateState::Confirming => waited = rebooting.map(|at| at.elapsed()),
                _ => {}
            }
            states.push(s)
        });
        assert!(waited.is_some_and(|w| w >= Duration::from_millis(100)));
        assert!(waited.is_some_and(|w| w < Duration::from_secs(2)));
        assert_eq!(result, Err("confirm timeout".into()));
    }
    struct Vanished {
        mock: crate::MockDeck,
        calls: u32,
    }
    impl Connector for Vanished {
        fn connect(&mut self) -> Result<(Session, crate::IncomingReceiver), String> {
            self.calls += 1;
            if self.calls > 1 {
                Err("unplugged".into())
            } else {
                Connector::connect(&mut self.mock)
            }
        }
    }
    #[test]
    fn port_never_returns_fails() {
        let mut connector = Vanished {
            mock: crate::MockDeck::new(),
            calls: 0,
        };
        assert_eq!(
            run(&mut connector, None, &bundle(), limits(), &mut |_| {}),
            Err("port return timeout".into())
        );
    }
}
