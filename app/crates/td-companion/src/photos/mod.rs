//! The photo frame: each Device's picture folder on this PC, its idle settings and clock, and the
//! stream that sends it pictures while it is idle, as `schema/protocol/link-v2.md` § "Photo
//! frame, idle, and clock" defines them.
pub(crate) mod clock;
pub(crate) mod encode;
pub(crate) mod folder;
mod stream;

use crate::{
    devices::{Capabilities, DeviceId, DeviceManager, LinkState},
    link::Links,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Weak},
    thread,
    time::{Duration, Instant},
};
use stream::{Streamer, Target};
use td_link::{
    LinkError,
    photo::{self, IdleSettings, PhotoError, PhotoShown},
};

const TICK: Duration = Duration::from_millis(500);
const CLOCK_EVERY: Duration = Duration::from_secs(600);
const RETRY_AFTER: Duration = Duration::from_secs(5);
const SLOT_WAIT: Duration = Duration::from_millis(250);
const SLICE: Duration = Duration::from_millis(100);

/// Whether a Device's Firmware shows pictures, and whether it has a clock.
pub fn supports(caps: Option<&Capabilities>) -> (bool, bool) {
    let has = |kind: &str| caps.is_some_and(|c| c.tile_kinds.iter().any(|k| k == kind));
    (
        has("photo-frame") && caps.is_some_and(|c| c.photo_bytes > 0),
        has("clock"),
    )
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    folders: BTreeMap<String, String>,
}

struct Conn {
    since_ms: u64,
    target: Option<Target>,
    idle: Option<IdleSettings>,
    idle_tried: Option<Instant>,
    clock: Option<(Instant, Option<i32>)>,
    worker: Option<u64>,
}

impl Conn {
    fn new(since_ms: u64) -> Self {
        Self {
            since_ms,
            target: None,
            idle: None,
            idle_tried: None,
            clock: None,
            worker: None,
        }
    }
}

#[derive(Default)]
struct State {
    folders: BTreeMap<String, String>,
    conns: HashMap<DeviceId, Conn>,
    generation: u64,
}

struct Inner {
    stopped: std::sync::atomic::AtomicBool,
    file: Option<PathBuf>,
    devices: DeviceManager,
    links: Links,
    state: Mutex<State>,
}

/// Cloning shares the same service.
#[derive(Clone)]
pub struct PhotoFrames {
    inner: Arc<Inner>,
}

impl PhotoFrames {
    /// Loads the folders from `file`, `photo-frame.json`, and starts the supervisor. `None`
    /// keeps them in memory only.
    pub(crate) fn new(file: Option<PathBuf>, devices: DeviceManager, links: Links) -> Self {
        let folders = file
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .unwrap_or_default()
            .folders;
        let inner = Arc::new(Inner {
            stopped: Default::default(),
            file,
            devices,
            links,
            state: Mutex::new(State {
                folders,
                ..State::default()
            }),
        });
        let weak = Arc::downgrade(&inner);
        thread::Builder::new()
            .name("td-photo-frames".into())
            .spawn(move || supervise(&weak))
            .expect("the photo frame thread starts");
        Self { inner }
    }

    pub(crate) fn stop(&self) {
        self.inner
            .stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The Device's picture folder on this PC.
    pub fn folder(&self, id: &DeviceId) -> Option<String> {
        self.inner.state.lock().folders.get(id.as_str()).cloned()
    }

    /// Sets or clears the folder and saves it. Blocking.
    pub fn set_folder(&self, id: &DeviceId, folder: Option<String>) -> std::io::Result<()> {
        let mut state = self.inner.state.lock();
        let mut folders = state.folders.clone();
        match folder {
            Some(folder) => folders.insert(id.to_string(), folder),
            None => folders.remove(id.as_str()),
        };
        if let Some(path) = &self.inner.file {
            let saved = Saved {
                folders: folders.clone(),
            };
            td_core::app_dirs::write_atomic(
                path,
                &serde_json::to_vec_pretty(&saved).expect("folders serialize"),
            )?;
        }
        state.folders = folders;
        Ok(())
    }

    /// Records idle settings the Device just reported or stored.
    pub fn idle_changed(&self, id: &DeviceId, idle: IdleSettings) {
        if let Some(conn) = self.inner.state.lock().conns.get_mut(id) {
            conn.idle = Some(idle);
        }
    }
}

impl Inner {
    fn plan(&self, id: &DeviceId, since_ms: u64, generation: u64) -> Option<(PathBuf, Target)> {
        let state = self.state.lock();
        let conn = state.conns.get(id)?;
        let idle = conn.idle?;
        let folder = state.folders.get(id.as_str())?;
        (conn.since_ms == since_ms && conn.worker == Some(generation) && idle.enabled)
            .then_some(())?;
        let target = Target {
            interval: Duration::from_secs(u64::from(idle.interval_s)),
            ..conn.target?
        };
        Some((PathBuf::from(folder), target))
    }

    fn release(&self, id: &DeviceId, generation: u64) {
        if let Some(conn) = self.state.lock().conns.get_mut(id)
            && conn.worker == Some(generation)
        {
            conn.worker = None;
        }
    }
}

fn supervise(weak: &Weak<Inner>) {
    loop {
        thread::sleep(TICK);
        let Some(inner) = weak.upgrade() else {
            return;
        };
        if inner.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        tick(&inner, weak);
    }
}

fn tick(inner: &Inner, weak: &Weak<Inner>) {
    let mut seen = HashSet::new();
    for device in inner.devices.summaries() {
        let LinkState::Connected { since_ms, .. } = device.link else {
            continue;
        };
        let (pictures, has_clock) = supports(device.capabilities.as_ref());
        if !pictures && !has_clock {
            continue;
        }
        let id = device.id.clone();
        seen.insert(id.clone());
        let offset = clock::offset_min();
        let (clock_due, idle_due) = {
            let mut state = inner.state.lock();
            let conn = state
                .conns
                .entry(id.clone())
                .or_insert_with(|| Conn::new(since_ms));
            if conn.since_ms != since_ms {
                *conn = Conn::new(since_ms);
            }
            conn.target = device.capabilities.as_ref().map(|caps| Target {
                screen: caps.screen,
                limit: caps.photo_bytes as usize,
                interval: Duration::ZERO,
            });
            let clock_due = has_clock
                && conn.clock.is_none_or(|(at, set)| match set {
                    Some(set) => set != offset || at.elapsed() >= CLOCK_EVERY,
                    None => at.elapsed() >= RETRY_AFTER,
                });
            let idle_due = pictures
                && conn.idle.is_none()
                && conn.idle_tried.is_none_or(|at| at.elapsed() >= RETRY_AFTER);
            if clock_due {
                conn.clock = Some((Instant::now(), None));
            }
            if idle_due {
                conn.idle_tried = Some(Instant::now());
            }
            (clock_due, idle_due)
        };
        if (clock_due || idle_due)
            && let Ok(session) = inner.devices.session(&id)
        {
            let clock_set = (clock_due
                && photo::clock_set(&session, clock::unix_s(), offset).is_ok())
            .then_some(offset);
            let idle = idle_due.then(|| photo::idle_get(&session).ok()).flatten();
            let mut state = inner.state.lock();
            if let Some(conn) = state.conns.get_mut(&id) {
                if let Some(offset) = clock_set {
                    conn.clock = Some((Instant::now(), Some(offset)));
                }
                if idle.is_some() {
                    conn.idle = idle;
                }
            }
        }
        let mut state = inner.state.lock();
        let wanted = state.folders.contains_key(id.as_str());
        state.generation += 1;
        let generation = state.generation;
        let Some(conn) = state.conns.get_mut(&id) else {
            continue;
        };
        if pictures && wanted && conn.worker.is_none() && conn.idle.is_some_and(|i| i.enabled) {
            conn.worker = Some(generation);
            let weak = weak.clone();
            thread::Builder::new()
                .name(format!("td-photos-{id}"))
                .spawn(move || work(&weak, &id, since_ms, generation))
                .expect("the photo stream thread starts");
        }
    }
    inner.state.lock().conns.retain(|id, _| seen.contains(id));
}

fn work(weak: &Weak<Inner>, id: &DeviceId, since_ms: u64, generation: u64) {
    let current = || {
        weak.upgrade()
            .filter(|inner| !inner.stopped.load(std::sync::atomic::Ordering::SeqCst))
            .and_then(|inner| inner.plan(id, since_ms, generation))
    };
    let mut streamer: Option<Streamer> = None;
    while let Some((folder, target)) = current() {
        if streamer.as_ref().is_none_or(|s| s.folder() != folder) {
            streamer = Some(Streamer::new(folder));
        }
        let stream = streamer.as_mut().expect("the stream exists");
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let wait = stream.step(&target, |jpeg| send(&inner, id, jpeg, target.limit));
        drop(inner);
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            thread::sleep(SLICE.min(left));
            if current().is_none_or(|(folder, _)| folder != stream.folder()) {
                break;
            }
        }
    }
    if let Some(inner) = weak.upgrade() {
        inner.release(id, generation);
    }
}

fn send(inner: &Inner, id: &DeviceId, jpeg: &[u8], limit: usize) -> Result<PhotoShown, PhotoError> {
    match inner.links.within(id.as_str(), SLOT_WAIT, |session| {
        photo::put_photo(session, jpeg, limit)
    }) {
        None => Err(PhotoError::Link(LinkError::TransferBusy)),
        Some(Ok(shown)) => shown,
        Some(Err(error)) => Err(PhotoError::Link(LinkError::Closed(error.to_string()))),
    }
}
