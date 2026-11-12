//! Live values for Live tiles. The Companion reads the Integration streams that connected Devices' Layouts bind and
//! sends each Device what changed over the link, never through the API ([ADR 0004](../../../../../docs/adr/0004-firmware-draws-live-tiles.md)).

mod covers;
mod router;

use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use td_core::live::LiveValue;
use td_host::integrations::{Delivery, LiveSample};

pub use router::{Binding, LiveCaps, Report, Router, StreamKey, bindings, stream_source};

use crate::devices::{DeviceManager, LinkState};
use crate::integrations::{Controls, Service};
use crate::layouts::LayoutAuthority;
use crate::link::Links;
use covers::CoverCache;
use router::{CoverJob, Prepared};

/// How often the router looks for new samples when nothing wakes it. A polled sample reaches its Devices within one
/// round.
const ROUND: Duration = Duration::from_millis(250);
/// How long a round waits for a Device whose link is busy, such as with a Layout push, before it tries next round.
const LINK_WAIT: Duration = Duration::from_millis(50);

struct Shared {
    stopped: std::sync::atomic::AtomicBool,
    authority: LayoutAuthority,
    wake: Mutex<bool>,
    woken: Condvar,
    router: Mutex<Router>,
    /// The newest polled samples, kept for rounds when the Integrations are busy.
    samples: Mutex<HashMap<StreamKey, LiveSample>>,
    covers: Mutex<Prepared>,
    /// Requests each Device accepted per Integration, for the Integration's delivery status.
    sent: Mutex<HashMap<(String, String), u64>>,
    #[cfg(any(test, feature = "test-hooks"))]
    injected: Mutex<HashMap<StreamKey, LiveSample>>,
    #[cfg(any(test, feature = "test-hooks"))]
    rounds: std::sync::atomic::AtomicU64,
}

impl Shared {
    fn new(authority: LayoutAuthority) -> Self {
        Self {
            stopped: Default::default(),
            authority,
            wake: Mutex::default(),
            woken: Condvar::new(),
            router: Mutex::default(),
            samples: Mutex::default(),
            covers: Mutex::default(),
            sent: Mutex::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            injected: Mutex::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            rounds: Default::default(),
        }
    }
    fn wake(&self) {
        *self.wake.lock() = true;
        self.woken.notify_one();
    }
}

#[derive(Clone)]
pub struct Live(Arc<Shared>);

impl Live {
    pub(crate) fn stop(&self) {
        self.0
            .stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.0.wake();
    }

    pub(crate) fn cover_assets(&self, device: &str) -> BTreeSet<String> {
        self.0.router.lock().cover_assets(device)
    }

    pub(crate) fn forget_cover(&self, device: &str, id: &str) {
        self.0.router.lock().forget_cover(device, id);
    }

    pub fn start(
        devices: DeviceManager,
        links: Links,
        integrations: Service,
        authority: LayoutAuthority,
    ) -> Live {
        let live = Live(Arc::new(Shared::new(authority)));
        let shared = Arc::downgrade(&live.0);
        live.0.authority.on_change(Arc::new(move || {
            if let Some(shared) = shared.upgrade() {
                shared.wake();
            }
        }));
        live.0
            .router
            .lock()
            .set_controls(integrations.controls.clone());
        for control in integrations.controls.values() {
            let shared = Arc::downgrade(&live.0);
            control.subscribe(Arc::new(move || {
                if let Some(shared) = shared.upgrade() {
                    shared.wake();
                }
            }));
        }
        let (jobs, queue) = sync_channel(1);
        let shared = Arc::downgrade(&live.0);
        let controls = integrations.controls.clone();
        thread::spawn(move || make_covers(shared, controls, queue));
        let shared = Arc::downgrade(&live.0);
        thread::spawn(move || run(shared, devices, links, integrations, jobs));
        live
    }

    /// Adds a sample as if an Integration reported it, for tests without real Integration values.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn inject(&self, key: StreamKey, sample: LiveSample) {
        self.0.injected.lock().insert(key, sample);
        self.0.wake();
    }

    /// How many live rounds have finished.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn rounds(&self) -> u64 {
        self.0.rounds.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn run(
    shared: Weak<Shared>,
    devices: DeviceManager,
    links: Links,
    integrations: Service,
    jobs: SyncSender<Vec<CoverJob>>,
) {
    loop {
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if shared.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        {
            let mut wake = shared.wake.lock();
            if !*wake {
                shared.woken.wait_for(&mut wake, ROUND);
            }
            *wake = false;
        }
        let covers = round(&shared, &devices, &links, &integrations);
        #[cfg(any(test, feature = "test-hooks"))]
        shared
            .rounds
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = jobs.try_send(covers);
    }
}

fn round(
    shared: &Shared,
    devices: &DeviceManager,
    links: &Links,
    integrations: &Service,
) -> Vec<CoverJob> {
    let (generation, changed) = shared.authority.begin_round();
    let connected: Vec<(String, u64, LiveCaps)> = devices
        .summaries()
        .into_iter()
        .filter_map(|device| match device.link {
            LinkState::Connected {
                since_ms,
                protocol: 2,
                ..
            } => {
                let caps = LiveCaps::of(device.capabilities.as_ref()?)?;
                Some((device.id.as_str().to_owned(), since_ms, caps))
            }
            _ => None,
        })
        .collect();
    let covers = shared.covers.lock().clone();
    let wanted = {
        let mut router = shared.router.lock();
        router.set_covers(covers);
        router.set_off(
            integrations
                .summaries()
                .into_iter()
                .filter(|summary| !summary.enabled)
                .map(|summary| summary.id)
                .collect(),
        );
        router.retain(&connected.iter().map(|(id, ..)| id.clone()).collect());
        for device in changed {
            router.layout_changed(&device);
        }
        router.wanted()
    };
    if let Some(read) = integrations.live(&wanted) {
        *shared.samples.lock() = read;
    }
    let mut samples = shared.samples.lock().clone();
    samples.extend(integrations.control_live(&wanted));
    #[cfg(any(test, feature = "test-hooks"))]
    samples.extend(shared.injected.lock().clone());
    for (device, session, caps) in &connected {
        let delivered = links.within(device, LINK_WAIT, |link| {
            shared
                .router
                .lock()
                .deliver(device, *session, link, *caps, &samples, Instant::now())
        });
        if let Some(Ok(report)) = delivered {
            record(shared, integrations, device, report);
        }
    }
    let (jobs, shown, layouts) = {
        let router = shared.router.lock();
        (
            router.cover_jobs(&samples),
            router.shown(),
            router.layouts(),
        )
    };
    let failed = connected
        .iter()
        .filter(|(id, session, _)| shared.router.lock().read_failed(id, *session))
        .map(|(id, session, _)| (id.clone(), *session))
        .collect();
    shared.authority.publish(generation, shown, layouts, failed);
    jobs
}

fn make_covers(shared: Weak<Shared>, controls: Controls, queue: Receiver<Vec<CoverJob>>) {
    let mut cache = CoverCache::default();
    while let Ok(jobs) = queue.recv() {
        let mut made = Prepared::new();
        for job in jobs {
            let Some(control) = controls.get(&job.source.integration) else {
                continue;
            };
            let LiveValue::Image(Some(source)) = &job.sample.value else {
                continue;
            };
            let still_reported = || control.live().contains(&job.sample);
            let cover = match cache.get(source, job.side) {
                Some(cover) => cover.ok(),
                None if !still_reported() => continue,
                None => control
                    .picture(source)
                    .and_then(|bytes| cache.prepare(source, &bytes, job.side).ok()),
            };
            if still_reported() {
                made.insert((job.source, job.sample.sample, job.side), cover);
            }
        }
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if shared.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        let mut covers = shared.covers.lock();
        if *covers != made {
            *covers = made;
            drop(covers);
            shared.wake();
        }
    }
}

fn record(shared: &Shared, integrations: &Service, device: &str, report: Report) {
    let touched: BTreeSet<&String> = report.sent.keys().chain(report.errors.keys()).collect();
    for integration in touched {
        let sent = {
            let mut totals = shared.sent.lock();
            let total = totals
                .entry((integration.clone(), device.to_owned()))
                .or_default();
            *total += report.sent.get(integration).copied().unwrap_or(0);
            *total
        };
        integrations.delivered(
            integration,
            Delivery {
                device: device.to_owned(),
                sent,
                error: report.errors.get(integration).cloned(),
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
