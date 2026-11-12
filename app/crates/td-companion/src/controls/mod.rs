//! Owns Device control input from admission through execution and gesture logging.
mod input;
use crate::actions::{ActionSpec, ApprovalRequest};
use crate::api::Events;
use crate::devices::{DeviceConsole, DeviceId};
use crate::integrations::Controls;
use crate::layouts::LayoutAuthority;
use crate::live::{StreamKey, stream_source};
use input::{Input, InputHub, PressGate};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};
use td_core::layout::{StreamType, TileRender};
use td_host::{actions::Plan, integrations::ControlError};
const INPUT_PACE: Duration = Duration::from_millis(20);
pub(crate) const ACT_TIMEOUT: Duration = Duration::from_secs(2);
const UNAVAILABLE: &str = "This Tile's control isn't available.";
const GESTURE_DONE: &str = "Volume changed.";

pub(crate) fn takes_input(tile: &td_core::layout::Tile, stream: &str) -> bool {
    tile.input
        && tile.render == Some(TileRender::Bar)
        && tile
            .streams
            .as_ref()
            .and_then(|streams| streams.get(stream))
            == Some(&StreamType::Number)
}

pub(crate) type Settable = Arc<dyn Fn(&StreamKey) -> bool + Send + Sync>;
pub(crate) type TurnedOn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

type Worker = Box<dyn FnOnce() + Send>;
type Spawn = Arc<dyn Fn(Worker) + Send + Sync>;
type Log = Arc<dyn Fn(&ApprovalRequest, &str, String) + Send + Sync>;
struct Execution {
    authority: LayoutAuthority,
    controls: Controls,
    settable: Settable,
    turned_on: TurnedOn,
    active: Arc<Mutex<std::collections::BTreeSet<String>>>,
    log: Log,
}
/// Two independent execution lanes for each connected Device.
pub(crate) struct DeviceControls {
    execution: Arc<Execution>,
    spawn: Spawn,
    events: Events,
    lanes: Mutex<HashMap<DeviceId, Lanes>>,
}
struct Lanes {
    session: u64,
    parts: PartSender,
    gate: PressGate,
    dropped: u64,
    drags: mpsc::SyncSender<()>,
    hub: Arc<Mutex<InputHub>>,
}
impl DeviceControls {
    pub(crate) fn new(
        authority: LayoutAuthority,
        controls: Controls,
        settable: Settable,
        turned_on: TurnedOn,
        events: Events,
        log: Log,
    ) -> Self {
        Self {
            spawn: Arc::new(|work| {
                thread::spawn(work);
            }),
            execution: Arc::new(Execution {
                authority,
                controls,
                settable,
                turned_on,
                active: Arc::default(),
                log,
            }),
            events,
            lanes: Mutex::default(),
        }
    }
    pub(crate) fn event(&self, device: &DeviceId, session: u64, event: &serde_json::Value) {
        let mut lanes = self.lanes.lock();
        if lanes
            .get(device)
            .is_some_and(|lane| lane.session != session)
        {
            lanes.remove(device);
        }
        let lane = lanes.entry(device.clone()).or_insert_with(|| {
            let (parts, taps) = PartSender::channel();
            let execution = self.execution.clone();
            let id = device.clone();
            (self.spawn)(Box::new(move || {
                for tap in taps {
                    execution.part(&id, session, &tap.value);
                }
            }));
            let (drags, inputs) = mpsc::sync_channel(1);
            let hub = Arc::new(Mutex::new(InputHub::default()));
            let worker_hub = hub.clone();
            let execution = self.execution.clone();
            let id = device.clone();
            (self.spawn)(Box::new(move || {
                run_inputs(&execution, session, &id, &inputs, &worker_hub)
            }));
            Lanes {
                session,
                parts,
                gate: PressGate::default(),
                dropped: 0,
                drags,
                hub,
            }
        });
        if event["m"] == "press" && event.get("part").is_some() {
            if !lane.gate.accept(event, Instant::now()) || !lane.parts.try_send(event.clone()) {
                lane.dropped = lane.dropped.saturating_add(1);
            }
        } else if event["m"] == "v2.input" {
            let mut hub = lane.hub.lock();
            hub.reset(self.execution.authority.generation(device));
            hub.push(event);
            let _ = lane.drags.try_send(());
        }
    }
    pub(crate) fn overflow(&self, device: &DeviceId) {
        if let Some(lane) = self.lanes.lock().get(device) {
            lane.hub.lock().cancel();
        }
    }
    pub(crate) fn closed(&self, device: &DeviceId) {
        if let Some(lane) = self.lanes.lock().remove(device)
            && lane.dropped > 0
        {
            self.events.publish(&DeviceConsole {
                device: device.clone(),
                line: format!("Dropped control taps: {}", lane.dropped),
            });
        }
    }
}
impl Execution {
    fn part(&self, device: &DeviceId, session: u64, event: &serde_json::Value) {
        let text = |name: &str| event[name].as_str().unwrap_or_default();
        let (page, tile, part) = (text("page"), text("tile"), text("part"));
        let wiring = self;
        let request = integration_request(device, page, tile);
        let found = wiring.authority.tile(device, session, page, tile);
        let Some(Plan::Integration {
            source,
            account,
            action,
        }) = found.and_then(|found| Plan::part(&found, part))
        else {
            return (self.log)(&request, "failed", UNAVAILABLE.into());
        };
        let Some(control) = wiring.controls.get(&source) else {
            return (self.log)(&request, "failed", UNAVAILABLE.into());
        };
        if !wiring.active.lock().insert(device.to_string()) {
            return (self.log)(&request, "failed", UNAVAILABLE.into());
        }
        let control = control.clone();
        let active = wiring.active.clone();
        let device = device.to_string();
        let operation = action.clone();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = control.act(&account, &operation, ACT_TIMEOUT);
            active.lock().remove(&device);
            let _ = send.send(result);
        });
        match receive
            .recv_timeout(ACT_TIMEOUT)
            .unwrap_or(Err(ControlError::Unavailable(
                "The player didn't respond.".into(),
            ))) {
            Ok(()) => (self.log)(&request, "ok", done(&action).into()),
            Err(error) => (self.log)(&request, "failed", detail(error)),
        }
    }

    fn set(&self, device: &DeviceId, session: u64, input: &Input) -> Option<Result<(), String>> {
        let wiring = self;
        let tile = match wiring
            .authority
            .binding(device, session, &input.page, &input.tile)
        {
            Ok(tile) => tile?,
            Err(()) => return Some(Err(UNAVAILABLE.into())),
        };
        let takes_input = takes_input(&tile, &input.stream);
        if !takes_input {
            return None;
        }
        let source = stream_source(&tile, &input.stream)?;
        if !(wiring.turned_on)(&source.integration) {
            return None;
        }
        let control = wiring.controls.get(&source.integration);
        let Some(control) = control.filter(|_| (wiring.settable)(&source)) else {
            return Some(Err(UNAVAILABLE.into()));
        };
        Some(
            control
                .set(&source.account, &source.stream, input.value)
                .map_err(detail),
        )
    }

    fn gesture(&self, device: &DeviceId, input: &Input, outcome: Result<(), String>) {
        let request = integration_request(device, &input.page, &input.tile);
        match outcome {
            Ok(()) => (self.log)(&request, "ok", GESTURE_DONE.into()),
            Err(reason) => (self.log)(&request, "failed", reason),
        }
    }
}
fn integration_request(device: &DeviceId, page: &str, tile: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: String::new(),
        device: device.clone(),
        page: page.into(),
        tile: tile.into(),
        hash: String::new(),
        spec: ActionSpec {
            kind: "integration".into(),
            program: String::new(),
            args: vec![],
            cwd: String::new(),
            shell: false,
        },
        created_ms: crate::now_ms(),
    }
}

fn done(action: &str) -> &'static str {
    match action {
        "mute" => "Mute changed.",
        "previous" => "Previous track.",
        "next" => "Next track.",
        _ => "Playback changed.",
    }
}

fn detail(error: ControlError) -> String {
    match error {
        ControlError::Unavailable(words) if !words.is_empty() => words,
        _ => UNAVAILABLE.into(),
    }
}

fn run_inputs(
    handler: &Execution,
    session: u64,
    id: &DeviceId,
    inputs: &Receiver<()>,
    hub: &parking_lot::Mutex<InputHub>,
) {
    while inputs.recv().is_ok() {
        thread::sleep(INPUT_PACE);
        let _ = inputs.try_recv();
        {
            let generation = handler.authority.generation(id);
            let batch = {
                let mut hub = hub.lock();
                hub.reset(generation);
                hub.take()
            };
            for input in batch {
                if handler.authority.generation(id) != generation || !hub.lock().is_current(&input)
                {
                    break;
                }
                let Some(result) = handler.set(id, session, &input) else {
                    hub.lock().dropped(&input);
                    continue;
                };
                let outcome = {
                    let mut hub = hub.lock();
                    hub.reset(handler.authority.generation(id));
                    if handler.authority.generation(id) != generation {
                        break;
                    }
                    hub.settled(&input, result, Instant::now())
                };
                if let Some(outcome) = outcome {
                    handler.gesture(id, &input, outcome);
                }
            }
        }
    }
}

struct PartTap {
    value: serde_json::Value,
    busy: Arc<AtomicBool>,
}

impl Drop for PartTap {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

struct PartSender {
    send: mpsc::SyncSender<PartTap>,
    busy: Arc<AtomicBool>,
}

impl PartSender {
    fn channel() -> (Self, Receiver<PartTap>) {
        let (send, receive) = mpsc::sync_channel(1);
        (
            Self {
                send,
                busy: Arc::default(),
            },
            receive,
        )
    }

    fn try_send(&self, value: serde_json::Value) -> bool {
        if self.busy.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.send
            .try_send(PartTap {
                value,
                busy: self.busy.clone(),
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod scheduling_tests {
    use super::*;
    #[test]
    fn a_part_waits_for_worker_scheduling_but_never_queues_behind_another_part() {
        let (parts, receive) = PartSender::channel();
        let first = serde_json::json!({"part": "next"});
        let second = serde_json::json!({"part": "previous"});
        assert!(parts.try_send(first.clone()));
        assert!(!parts.try_send(second.clone()));
        let running = receive.try_recv().unwrap();
        assert_eq!(running.value, first);
        assert!(!parts.try_send(second.clone()));
        drop(running);
        assert!(receive.try_recv().is_err());
        assert!(parts.try_send(second.clone()));
        assert_eq!(receive.try_recv().unwrap().value, second);
    }

    #[test]
    fn a_stopped_part_worker_refuses_taps() {
        let (parts, receive) = PartSender::channel();
        drop(receive);
        assert!(!parts.try_send(serde_json::json!({"part": "next"})));
        assert!(!parts.busy.load(Ordering::Acquire));
    }
}
#[cfg(test)]
mod lifecycle_tests;
