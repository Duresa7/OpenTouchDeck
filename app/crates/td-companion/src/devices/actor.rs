//! One Device's actor. Its work thread owns the link session and runs requests one at a
//! time; its forwarder thread turns what the Device sends into events and mirrored keys.
//! Layout and Asset work borrows the same session (see `crate::link`), since the port can
//! only be open once.
use super::{
    Carrier, DeviceConsole, DeviceEvent, DeviceId, DeviceKeyMirror, Inner, InputFactory,
    KeyMirrorState,
};
use std::{
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread,
    time::Instant,
};
use td_host::{Mirror, Mirrored, Outcome};
use td_link::{Incoming, Session};

pub type Work = Box<dyn FnOnce(&Session) + Send>;

/// Dropping the handle ends the work thread, which closes the session and frees the port.
pub(super) struct Handle {
    pub connection: u64,
    pub carrier: Carrier,
    pub opened: Instant,
    work: Sender<Work>,
    session: Arc<Session>,
}
impl Handle {
    pub(super) fn sender(&self) -> Sender<Work> {
        self.work.clone()
    }
    /// The live session. It stops answering once the work thread closes it.
    pub(super) fn session(&self) -> Arc<Session> {
        self.session.clone()
    }
}

pub(super) fn spawn(
    inner: Arc<Inner>,
    id: DeviceId,
    (connection, carrier): (u64, Carrier),
    session_id: u64,
    session: Session,
    incoming: td_link::IncomingReceiver,
    mirror: Option<Mirror>,
) -> Handle {
    let (work, jobs) = mpsc::channel::<Work>();
    let session = Arc::new(session);
    let shared = session.clone();
    let (presses, taps) = mpsc::sync_channel::<serde_json::Value>(16);
    let action_inner = inner.clone();
    let action_id = id.clone();
    inner.track(thread::spawn(move || {
        for event in taps {
            if let Some(handler) = action_inner.actions.get() {
                handler(&action_id, &event);
            }
        }
    }));
    inner.track(
        thread::Builder::new()
            .name(format!("td-device-{id}"))
            .spawn(move || {
                for job in jobs {
                    job(&session);
                }
                session.close();
            })
            .expect("the Device thread starts"),
    );
    let forward_inner = inner.clone();
    inner.track(
        thread::Builder::new()
            .name(format!("td-device-{id}-in"))
            .spawn(move || {
                forward(
                    &forward_inner,
                    &id,
                    connection,
                    session_id,
                    incoming,
                    mirror,
                    Touches { presses },
                )
            })
            .expect("the Device forwarder starts"),
    );
    Handle {
        connection,
        carrier,
        opened: Instant::now(),
        work,
        session: shared,
    }
}

/// A key mirror for a Device, unless it types through its own USB HID or the PC has no sink.
pub(super) fn mirror(
    input: &InputFactory,
    hello: &td_core::Hello,
) -> (Option<Mirror>, KeyMirrorState) {
    match input() {
        Ok(sink) => {
            let mut mirror = Mirror::new(sink);
            let wanted = Mirror::wanted(&hello.sinks);
            mirror.set_enabled(wanted);
            let reason = if wanted {
                "Typing the Device's keys on this PC."
            } else {
                "Off: the Device types through its own USB HID."
            };
            let state = KeyMirrorState {
                enabled: wanted,
                reason: Some(reason.into()),
            };
            (Some(mirror), state)
        }
        Err(error) => {
            let state = KeyMirrorState {
                enabled: false,
                reason: Some(format!("Off: {error}")),
            };
            (None, state)
        }
    }
}

struct Touches {
    presses: mpsc::SyncSender<serde_json::Value>,
}

fn forward(
    inner: &Inner,
    id: &DeviceId,
    connection: u64,
    session_id: u64,
    incoming: impl IntoIterator<Item = Incoming>,
    mut mirror: Option<Mirror>,
    touches: Touches,
) {
    for item in incoming {
        match item {
            Incoming::Line(line) => inner.events.publish(&DeviceConsole {
                device: id.clone(),
                line,
            }),
            Incoming::Event(event) => {
                if event["m"] == "press" {
                    if event.get("part").is_some() {
                        if let Some(controls) =
                            inner.controls.get().and_then(std::sync::Weak::upgrade)
                        {
                            controls.event(id, session_id, &event);
                        }
                    } else {
                        let _ = touches.presses.try_send(event.clone());
                    }
                } else if event["m"] == "v2.input" {
                    if let Some(controls) = inner.controls.get().and_then(std::sync::Weak::upgrade)
                    {
                        controls.event(id, session_id, &event);
                    }
                } else if event["m"] == "v2.page"
                    && let Some(page) = event["page"].as_str()
                {
                    inner.page_shown(id, connection, page);
                }
                if let Some(report) = Mirrored::parse(&event) {
                    key_report(inner, id, mirror.as_mut(), &report);
                }
                inner.events.publish(&DeviceEvent {
                    device: id.clone(),
                    event,
                });
            }
            Incoming::Overflow => {
                if let Some(mirror) = mirror.as_mut() {
                    mirror.release_all();
                }
                if let Some(controls) = inner.controls.get().and_then(std::sync::Weak::upgrade) {
                    controls.overflow(id);
                }
                inner.events.publish(&DeviceConsole {
                    device: id.clone(),
                    line: "Dropped Device events: overflow".into(),
                });
            }
            Incoming::Closed(reason) => {
                if let Some(controls) = inner.controls.get().and_then(std::sync::Weak::upgrade) {
                    controls.closed(id);
                }
                if let Some(mirror) = mirror.as_mut() {
                    mirror.release_all();
                }
                inner.closed(id, connection, &reason);
                return;
            }
        }
    }
}

/// Types one key report on the PC. While a dialog suppresses input, the report is dropped,
/// not replayed: the gap it leaves releases every held key when typing resumes.
fn key_report(inner: &Inner, id: &DeviceId, mirror: Option<&mut Mirror>, report: &Mirrored) {
    let _gate = inner.input_gate.lock();
    let arrived_ms = crate::now_ms();
    let (outcome, gap, errors) = match mirror {
        _ if inner.input_suppressed() => ("suppressed", None, Vec::new()),
        None => ("off", None, Vec::new()),
        Some(mirror) => {
            let outcome = mirror.apply(report);
            let errors = std::mem::take(&mut mirror.errors);
            match outcome {
                Outcome::Typed { gap } => ("typed", gap.map(|(a, b)| [a, b]), errors),
                Outcome::Duplicate => ("duplicate", None, errors),
                Outcome::Off => ("off", None, errors),
                Outcome::Unknown => ("unknown", None, errors),
            }
        }
    };
    inner.events.publish(&DeviceKeyMirror {
        device: id.clone(),
        seq: report.seq(),
        report: report.describe(),
        outcome: outcome.into(),
        gap,
        errors,
        arrived_ms,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_reports_type_nothing_once_the_devices_stop() {
        // A Device thread left running after a timed-out shutdown must not type on the PC.
        let devices = super::super::DeviceManager::new(crate::api::Events::default());
        let sink = td_host::FakeSink::default();
        let mut mirror = Mirror::new(Box::new(sink.clone()));
        mirror.set_enabled(true);
        devices.stop();
        let down = serde_json::json!({"m":"key.report","seq":1,"type":"kbd","mods":0,"keys":[4]});
        forward(
            &devices.inner,
            &DeviceId::parse("mock0001").unwrap(),
            0,
            0,
            [Incoming::Event(down)],
            Some(mirror),
            Touches {
                presses: mpsc::sync_channel(16).0,
            },
        );
        assert_eq!(sink.take(), Vec::new());
    }

    #[test]
    fn overflow_releases_held_keys_before_the_next_event() {
        let devices = super::super::DeviceManager::new(crate::api::Events::default());
        let sink = td_host::FakeSink::default();
        let mut mirror = Mirror::new(Box::new(sink.clone()));
        mirror.set_enabled(true);
        let down = serde_json::json!({"m":"key.report","seq":1,"type":"kbd","mods":1,"keys":[4,5]});
        let incoming = [
            Incoming::Event(down),
            Incoming::Overflow,
            Incoming::Line("after".into()),
        ];
        let incoming = incoming.into_iter().inspect(|item| {
            if matches!(item, Incoming::Line(_)) {
                let events = sink.take();
                assert_eq!(events.len(), 6);
                for event in &events[..3] {
                    let td_host::KeyEvent::Down(code) = event else {
                        panic!("expected down")
                    };
                    assert!(events[3..].contains(&td_host::KeyEvent::Up(*code)));
                }
            }
        });
        forward(
            &devices.inner,
            &DeviceId::parse("mock0001").unwrap(),
            0,
            0,
            incoming,
            Some(mirror),
            Touches {
                presses: mpsc::sync_channel(16).0,
            },
        );
    }
}
