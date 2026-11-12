//! Which Integration values each Device needs, and sending them over `v2.live.set`.
//!
//! The rules come from `schema/protocol/link-v2.md` § "Layout references and live values" and from
//! `docs/companion-api.md` § "Live tile bindings". The router is plain state: the caller hands it the Device's
//! session, its live caps and the newest Integration samples, so tests drive it with a mock Device.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use td_core::layout::{Action, Page, StreamType, Tile};
use td_core::live::{ScreenSize, art_size, tile_rect};
use td_core::{Layout, Orientation};
use td_host::integrations::LiveSample;
use td_link::asset::{AssetError, AssetReason};
use td_link::live::{self, LiveError, LiveReason, LiveSet, LiveValue};
use td_link::{Session, xfer};

use super::covers::{COVER_SIDE_MIN, Cover, CoverStore};
use crate::devices::{LayoutSource, ShownLayout};
use crate::features::integrations::Controls;

/// How long a Device's `layout state` reply may take.
const LAYOUT_STATE_TIMEOUT: Duration = Duration::from_secs(2);
/// The shortest gap between two values of one stream. A source that samples once a second lands just under a
/// second apart, so the gap leaves it room.
const STREAM_GAP: Duration = Duration::from_millis(900);
const CONTROL_GAP: Duration = Duration::from_millis(500);
/// How long the router waits to read a Device's active Layout again after a failed read.
const LAYOUT_RETRY: Duration = Duration::from_secs(5);
const LAYOUT_CHECK: Duration = Duration::from_secs(5);
/// The longest wait after repeated `rate` refusals.
const BACKOFF_MAX: Duration = Duration::from_secs(8);
const TITLE_STREAM: &str = "title";
const OUT_OF_SPACE: &str =
    "The Device is out of space for cover art. Remove unused Assets, then retry.";
const BUILTIN: &[u8] = include_bytes!("../../../../../schema/examples/builtin.json");

/// One Integration stream: the Integration's id, the account it reads (empty for one-account Integrations) and the
/// stream id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamKey {
    pub integration: String,
    pub account: String,
    pub stream: String,
}

/// One Tile stream of a Device's active Layout and the Integration stream that feeds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub page: String,
    pub tile: String,
    pub stream: String,
    pub kind: StreamType,
    pub source: StreamKey,
}

/// The Tile streams a Layout binds to Integration streams. A Tile binds through its `widget` action: `source` names
/// the Integration and `account` the account. Its `value` stream reads the Integration stream `field`, and any other
/// stream reads the Integration stream of the same id.
pub fn bindings(layout: &Layout) -> Vec<Binding> {
    let mut out = Vec::new();
    for page in &layout.pages {
        let Page::Grid {
            id: page, tiles, ..
        } = page
        else {
            continue;
        };
        for tile in tiles {
            let Action::Widget {
                source,
                field,
                account,
            } = &tile.action
            else {
                continue;
            };
            for (stream, kind) in tile.streams.iter().flatten() {
                out.push(Binding {
                    page: page.clone(),
                    tile: tile.id.clone(),
                    stream: stream.clone(),
                    kind: *kind,
                    source: source_of(source, account, field, stream),
                });
            }
        }
    }
    out
}

fn source_of(source: &str, account: &str, field: &str, stream: &str) -> StreamKey {
    StreamKey {
        integration: source.to_owned(),
        account: account.to_owned(),
        stream: if stream == "value" { field } else { stream }.to_owned(),
    }
}

/// The Integration stream one stream of `tile` reads, if the Tile binds an Integration through its `widget` action.
pub fn stream_source(tile: &Tile, stream: &str) -> Option<StreamKey> {
    let Action::Widget {
        source,
        field,
        account,
    } = &tile.action
    else {
        return None;
    };
    tile.streams
        .as_ref()?
        .contains_key(stream)
        .then(|| source_of(source, account, field, stream))
}

/// The live limits a Device advertised in `caps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveCaps {
    pub live_hz: u32,
    pub live_total_hz: u32,
    pub series: u32,
    pub text_bytes: u32,
    pub live_images: u32,
    pub image: [u32; 2],
    pub screen: [u32; 2],
}

impl LiveCaps {
    /// The caps of a Device that takes live values, or None.
    pub fn of(caps: &crate::devices::Capabilities) -> Option<LiveCaps> {
        let caps = LiveCaps {
            live_hz: caps.live_hz,
            live_total_hz: caps.live_total_hz,
            series: caps.series,
            text_bytes: caps.text_bytes,
            live_images: caps.live_images,
            image: caps.image,
            screen: caps.screen,
        };
        (caps.live_hz > 0 && caps.live_total_hz > 0).then_some(caps)
    }

    /// The shortest gap between two values of one stream: `max(500 ms, 1000 / live_hz)` for a Control's stream and
    /// `max(900 ms, 1000 / live_hz)` for a polled one.
    fn gap(&self, control: bool) -> Duration {
        let floor = if control { CONTROL_GAP } else { STREAM_GAP };
        floor.max(Duration::from_millis(u64::from(
            1000_u32.div_ceil(self.live_hz.max(1)),
        )))
    }

    /// The value as this Device can take it: a series cut to its newest `series` numbers and text folded and cut by
    /// `td_core::live::fit_text`. None when nothing of it fits.
    fn fit(&self, value: &LiveValue) -> Option<LiveValue> {
        if matches!(value, LiveValue::Image(_)) && self.live_images == 0 {
            return None;
        }
        let fitted = match value {
            LiveValue::Series(values) => {
                let keep = values.len().min(self.series as usize);
                LiveValue::Series(values[values.len() - keep..].to_vec())
            }
            LiveValue::Text(text) => {
                LiveValue::Text(td_core::live::fit_text(text, self.text_bytes as usize))
            }
            other => other.clone(),
        };
        fitted
            .fits(self.series as usize, self.text_bytes as usize)
            .then_some(fitted)
    }
}

/// What the router last did for one Tile stream.
#[derive(Debug, Default)]
struct Track {
    /// The sample the Device holds.
    delivered: Option<u64>,
    /// A send whose reply never came: its `n`, sample and value, so the same value goes again as an identical retry.
    unanswered: Option<(u32, u64, LiveValue)>,
    last_sent: Option<Instant>,
    /// The sample and value the Device last accepted, so the Device can be told it is no longer current.
    held: Option<(u64, LiveValue)>,
    /// The Device was told the held value is no longer current, so it shows it stale.
    expired: bool,
}

/// The Layout a Device shows and its bindings.
#[derive(Debug)]
struct Active {
    reference: String,
    bindings: Vec<Binding>,
    layout: Arc<Layout>,
    slots: Vec<(Orientation, Option<String>)>,
}

type TileKey = (String, String);

#[derive(Debug, Default)]
struct DeviceState {
    session: u64,
    caps: Option<LiveCaps>,
    active: Option<Active>,
    checked_at: Option<Instant>,
    read_failed: bool,
    tracks: HashMap<(String, String, String), Track>,
    next_n: u32,
    /// The cost of each request in the last second, oldest first.
    spent: VecDeque<(Instant, u32)>,
    blocked_until: Option<Instant>,
    rate_strikes: u32,
    covers: CoverStore,
    swept: bool,
    cover_retry: Option<Instant>,
    cover_failures: u32,
    titles: HashMap<TileKey, String>,
    covered: HashSet<TileKey>,
}

/// What one delivery round did on one Device, per Integration.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub sent: HashMap<String, u64>,
    pub errors: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CoverJob {
    pub source: StreamKey,
    pub sample: LiveSample,
    pub side: u16,
}

pub(super) type Prepared = HashMap<(StreamKey, u64, u16), Option<Arc<Cover>>>;

#[derive(Default)]
pub struct Router {
    devices: HashMap<String, DeviceState>,
    controls: Controls,
    covers: Prepared,
    off: BTreeSet<String>,
}

enum Sent {
    Accepted,
    AssetRefused,
    Refused,
    Stop,
}

impl Router {
    pub(super) fn cover_assets(&self, device: &str) -> BTreeSet<String> {
        self.devices
            .get(device)
            .into_iter()
            .flat_map(|state| state.tracks.values())
            .flat_map(|track| {
                track
                    .held
                    .as_ref()
                    .map(|(_, value)| value)
                    .into_iter()
                    .chain(track.unanswered.as_ref().map(|(_, _, value)| value))
            })
            .filter_map(|value| match value {
                LiveValue::Image(Some(reference)) => {
                    reference.strip_prefix("sha256:").map(str::to_owned)
                }
                _ => None,
            })
            .collect()
    }

    pub(super) fn forget_cover(&mut self, device: &str, id: &str) {
        if let Some(state) = self.devices.get_mut(device) {
            state.covers.forget(id);
        }
    }

    /// The Controls behind some Integrations: their streams use the Control gap and are rechecked before each send.
    pub fn set_controls(&mut self, controls: Controls) {
        self.controls = controls;
    }

    pub(super) fn set_covers(&mut self, covers: Prepared) {
        self.covers = covers;
    }

    /// The Integrations that are turned off. A stream of one that reports no sample is no longer current, so its
    /// Devices are told to show the last value stale.
    pub(super) fn set_off(&mut self, off: BTreeSet<String>) {
        self.off = off;
    }

    pub(super) fn cover_jobs(&self, samples: &HashMap<StreamKey, LiveSample>) -> Vec<CoverJob> {
        let mut jobs: Vec<CoverJob> = Vec::new();
        for state in self.devices.values() {
            let (Some(active), Some(caps)) = (&state.active, &state.caps) else {
                continue;
            };
            for binding in active
                .bindings
                .iter()
                .filter(|b| b.kind == StreamType::Image)
            {
                let Some(sample) = samples.get(&binding.source) else {
                    continue;
                };
                let (LiveValue::Image(Some(_)), Some(side)) =
                    (&sample.value, cover_side(&active.layout, binding, caps))
                else {
                    continue;
                };
                let job = CoverJob {
                    source: binding.source.clone(),
                    sample: sample.clone(),
                    side,
                };
                if !jobs.contains(&job) {
                    jobs.push(job);
                }
            }
        }
        jobs
    }

    pub(super) fn read_failed(&self, device: &str, session: u64) -> bool {
        self.devices
            .get(device)
            .is_some_and(|state| state.session == session && state.read_failed)
    }

    pub(super) fn shown(&self) -> HashMap<String, (u64, Arc<Layout>)> {
        self.devices
            .iter()
            .filter_map(|(device, state)| {
                let active = state.active.as_ref()?;
                Some((device.clone(), (state.session, active.layout.clone())))
            })
            .collect()
    }

    pub(super) fn layouts(&self) -> Vec<(String, u64, ShownLayout)> {
        self.devices
            .iter()
            .filter_map(|(device, state)| {
                let active = state.active.as_ref()?;
                let source = if active.reference == live::BUILTIN {
                    LayoutSource::Builtin
                } else {
                    LayoutSource::Storage
                };
                let shown = ShownLayout::new(&active.layout, source);
                Some((device.clone(), state.session, shown))
            })
            .collect()
    }

    /// The Integration streams some Device's active Layout binds. Only these are read from the Integrations.
    pub fn wanted(&self) -> BTreeSet<StreamKey> {
        self.devices
            .values()
            .filter_map(|device| device.active.as_ref())
            .flat_map(|active| active.bindings.iter().map(|b| b.source.clone()))
            .collect()
    }

    /// The Device showed a new Layout. Its bindings are read again and every value goes again.
    pub fn layout_changed(&mut self, device: &str) {
        if let Some(state) = self.devices.get_mut(device) {
            state.active = None;
            if std::mem::take(&mut state.read_failed) {
                state.blocked_until = None;
            }
        }
    }

    /// Drops the Devices that are no longer connected.
    pub fn retain(&mut self, connected: &BTreeSet<String>) {
        self.devices.retain(|id, _| connected.contains(id));
    }

    /// Sends one Device the values its Layout binds that changed since it last got them. `session` identifies the
    /// link session: a new one clears the Device's live values, so everything goes again.
    pub fn deliver(
        &mut self,
        device: &str,
        session_id: u64,
        session: &Session,
        caps: LiveCaps,
        samples: &HashMap<StreamKey, LiveSample>,
        now: Instant,
    ) -> Report {
        let state = self.devices.entry(device.to_owned()).or_default();
        if state.session != session_id {
            *state = DeviceState {
                session: session_id,
                next_n: state.next_n,
                ..DeviceState::default()
            };
        }
        state.caps = Some(caps);
        let mut report = Report::default();
        if state.blocked_until.is_some_and(|until| until > now) {
            return report;
        }
        let mut sweep_error = None;
        if caps.live_images > 0 && !state.swept && state.cover_retry.is_none_or(|at| now >= at) {
            match state.covers.sweep(session) {
                Ok(done) => {
                    state.swept = done;
                    state.cover_retry = None;
                    state.cover_failures = 0;
                }
                Err(error) => {
                    retry_cover(state, caps.gap(true), now);
                    sweep_error = Some(error.to_string());
                }
            }
        }
        let stale = match &state.active {
            Some(active)
                if state
                    .checked_at
                    .is_none_or(|at| now.duration_since(at) >= LAYOUT_CHECK) =>
            {
                state.checked_at = Some(now);
                !slots_unchanged(session, active)
            }
            _ => false,
        };
        if state.active.is_none() || stale {
            match read_active(session) {
                Ok(active) => {
                    state.read_failed = false;
                    state.checked_at = Some(now);
                    if state
                        .active
                        .as_ref()
                        .is_none_or(|shown| shown.reference != active.reference)
                    {
                        state.tracks.clear();
                        state.titles.clear();
                        state.covered.clear();
                    }
                    state.active = Some(active);
                }
                Err(_) => {
                    state.active = None;
                    state.read_failed = true;
                    state.blocked_until = Some(now + LAYOUT_RETRY);
                    return report;
                }
            }
        }
        while state
            .spent
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= Duration::from_secs(1))
        {
            state.spent.pop_front();
        }
        let mut budget = caps
            .live_total_hz
            .saturating_sub(state.spent.iter().map(|(_, cost)| cost).sum());
        let Some(active) = state.active.as_ref() else {
            return report;
        };
        let reference = active.reference.clone();
        let bindings = active.bindings.clone();
        let sides: HashMap<TileKey, Option<u16>> = bindings
            .iter()
            .filter(|b| b.kind == StreamType::Image)
            .map(|b| (tile_key(b), cover_side(&active.layout, b, &caps)))
            .collect();
        if let Some(error) = sweep_error {
            for binding in bindings.iter().filter(|b| b.kind == StreamType::Image) {
                report
                    .errors
                    .insert(binding.source.integration.clone(), error.clone());
            }
        }
        for binding in &bindings {
            if binding.kind == StreamType::Image
                || samples.contains_key(&binding.source)
                || !self.off.contains(&binding.source.integration)
            {
                continue;
            }
            let Some(track) = state.tracks.get(&key(binding)) else {
                continue;
            };
            let Some((sample, value)) = track.held.clone().filter(|_| !track.expired) else {
                continue;
            };
            let gap = caps.gap(self.controls.contains_key(&binding.source.integration));
            if track
                .last_sent
                .is_some_and(|at| now.duration_since(at) < gap)
            {
                continue;
            }
            let out = Outgoing {
                binding,
                sample,
                ttl_ms: live::TTL_MIN_MS,
                value: &value,
                delivered: None,
            };
            match send(
                state,
                session,
                &reference,
                out,
                now,
                &mut budget,
                &mut report,
            ) {
                Sent::Stop => return report,
                Sent::Accepted | Sent::AssetRefused | Sent::Refused => {
                    if let Some(track) = state.tracks.get_mut(&key(binding)) {
                        track.expired = true;
                    }
                }
            }
        }
        let mut due: Vec<(&Binding, &LiveSample)> = bindings
            .iter()
            .filter_map(|binding| {
                let sample = samples.get(&binding.source)?;
                let track = state.tracks.get(&key(binding));
                let fresh = track.is_none_or(|t| t.delivered != Some(sample.sample));
                let gap = caps.gap(self.controls.contains_key(&binding.source.integration));
                let rested = track
                    .and_then(|t| t.last_sent)
                    .is_none_or(|at| now.duration_since(at) >= gap);
                (fresh && (rested || matches!(sample.value, LiveValue::Image(Some(_)))))
                    .then_some((binding, sample))
            })
            .collect();
        due.sort_by_key(|(binding, _)| {
            (
                binding.kind == StreamType::Image,
                state.tracks.get(&key(binding)).and_then(|t| t.last_sent),
            )
        });
        let mut cleared: HashSet<TileKey> = HashSet::new();
        for (binding, sample) in due {
            if !still_reported(&self.controls, binding, sample) {
                continue;
            }
            let integration = binding.source.integration.clone();
            let tile = tile_key(binding);
            let Some(fitted) = caps
                .fit(&sample.value)
                .filter(|value| value.kind() == binding.kind)
            else {
                report.errors.insert(
                    integration,
                    format!(
                        "Tile \"{}\" takes a {} value",
                        binding.tile,
                        binding.kind.name()
                    ),
                );
                continue;
            };
            if binding.kind == StreamType::Image && cleared.contains(&tile) {
                continue;
            }
            let title_changed = bindings.iter().find(|b| tile_key(b) == tile && b.stream == TITLE_STREAM)
                .and_then(|b| samples.get(&b.source).and_then(|s| caps.fit(&s.value)))
                .is_some_and(|value| matches!(value, LiveValue::Text(ref title) if state.titles.get(&tile) != Some(title)));
            if title_changed
                && state.covered.contains(&tile)
                && let Some(cover) = bindings
                    .iter()
                    .find(|b| tile_key(b) == tile && b.kind == StreamType::Image)
            {
                let gap = caps.gap(self.controls.contains_key(&cover.source.integration));
                if state
                    .tracks
                    .get(&key(cover))
                    .and_then(|t| t.last_sent)
                    .is_some_and(|at| now.duration_since(at) < gap)
                {
                    continue;
                }
                let clear = LiveValue::Image(None);
                let cover_sample = samples.get(&cover.source);
                let cleared_sample = cover_sample.filter(|s| s.value == clear).map(|s| s.sample);
                let out = Outgoing {
                    binding: cover,
                    sample: cover_sample.map_or(0, |s| s.sample),
                    ttl_ms: sample.ttl_ms,
                    value: &clear,
                    delivered: cleared_sample,
                };
                match send(
                    state,
                    session,
                    &reference,
                    out,
                    now,
                    &mut budget,
                    &mut report,
                ) {
                    Sent::Accepted => {
                        state.covered.remove(&tile);
                        cleared.insert(tile.clone());
                    }
                    Sent::Stop => break,
                    Sent::AssetRefused | Sent::Refused => continue,
                }
            }
            if binding.kind == StreamType::Image && cleared.contains(&tile) {
                continue;
            }
            let mut delivered = Some(sample.sample);
            let value = match fitted {
                LiveValue::Image(Some(_)) => {
                    let side = sides.get(&tile).copied().flatten();
                    let prepared = side.map(|side| {
                        self.covers
                            .get(&(binding.source.clone(), sample.sample, side))
                            .cloned()
                    });
                    match prepared {
                        Some(Some(Some(cover))) => {
                            if !state.swept || state.cover_retry.is_some_and(|at| now < at) {
                                continue;
                            }
                            match state.covers.upload(session, &cover) {
                                Ok(true) => {
                                    state.cover_retry = None;
                                    state.cover_failures = 0;
                                    LiveValue::Image(Some(format!("sha256:{}", cover.id)))
                                }
                                Ok(false) => {
                                    delivered = None;
                                    retry_cover(state, caps.gap(true), now);
                                    LiveValue::Image(None)
                                }
                                Err(error) if error.reason() == Some(&AssetReason::Full) => {
                                    report
                                        .errors
                                        .insert(integration.clone(), OUT_OF_SPACE.into());
                                    LiveValue::Image(None)
                                }
                                Err(error) => {
                                    retry_cover(state, caps.gap(true), now);
                                    let link = matches!(error, AssetError::Link(_));
                                    report.errors.insert(integration.clone(), error.to_string());
                                    if link {
                                        break;
                                    }
                                    continue;
                                }
                            }
                        }
                        Some(None) => {
                            let shows_nothing = !state.covered.contains(&tile)
                                && state
                                    .tracks
                                    .get(&key(binding))
                                    .is_some_and(|t| t.last_sent.is_some());
                            if shows_nothing {
                                continue;
                            }
                            delivered = None;
                            LiveValue::Image(None)
                        }
                        None | Some(Some(None)) => LiveValue::Image(None),
                    }
                }
                other => other,
            };
            if !still_reported(&self.controls, binding, sample) {
                continue;
            }
            let gap = caps.gap(self.controls.contains_key(&binding.source.integration));
            if state
                .tracks
                .get(&key(binding))
                .and_then(|t| t.last_sent)
                .is_some_and(|at| now.duration_since(at) < gap)
            {
                continue;
            }
            let new_title = match &value {
                LiveValue::Text(title) if binding.stream == TITLE_STREAM => {
                    (state.titles.get(&tile) != Some(title)).then(|| title.clone())
                }
                _ => None,
            };
            let out = Outgoing {
                binding,
                sample: sample.sample,
                ttl_ms: sample.ttl_ms,
                value: &value,
                delivered,
            };
            match send(
                state,
                session,
                &reference,
                out,
                now,
                &mut budget,
                &mut report,
            ) {
                Sent::Accepted => {
                    if let Some(title) = new_title {
                        state.titles.insert(tile.clone(), title);
                    }
                    match &value {
                        LiveValue::Image(Some(_)) => {
                            state.covered.insert(tile);
                        }
                        LiveValue::Image(None) => {
                            state.covered.remove(&tile);
                        }
                        _ => {}
                    }
                }
                Sent::AssetRefused => {
                    if let LiveValue::Image(Some(reference)) = &value {
                        state
                            .covers
                            .forget(reference.strip_prefix("sha256:").unwrap_or(reference));
                    }
                }
                Sent::Refused => {}
                Sent::Stop => break,
            }
        }
        report
    }
}

fn retry_cover(state: &mut DeviceState, gap: Duration, now: Instant) {
    state.cover_failures = state.cover_failures.saturating_add(1);
    state.cover_retry =
        Some(now + (gap * 2_u32.pow(state.cover_failures.min(4) - 1)).min(BACKOFF_MAX));
}

struct Outgoing<'a> {
    binding: &'a Binding,
    sample: u64,
    ttl_ms: u32,
    value: &'a LiveValue,
    delivered: Option<u64>,
}

fn send(
    state: &mut DeviceState,
    session: &Session,
    reference: &str,
    out: Outgoing,
    now: Instant,
    budget: &mut u32,
    report: &mut Report,
) -> Sent {
    let Outgoing {
        binding,
        sample,
        ttl_ms,
        value,
        delivered,
    } = out;
    let integration = binding.source.integration.clone();
    let track = state.tracks.entry(key(binding)).or_default();
    let n = match &track.unanswered {
        Some((n, unanswered, retry)) if *unanswered == sample && retry == value => *n,
        _ => {
            state.next_n = state.next_n.saturating_add(1);
            state.next_n
        }
    };
    let set = LiveSet {
        layout: reference,
        page: &binding.page,
        tile: &binding.tile,
        stream: &binding.stream,
        n,
        ttl_ms: ttl_ms.clamp(live::TTL_MIN_MS, live::TTL_MAX_MS),
        value,
    };
    let cost = live::cost(set.to_payload().len());
    if cost > *budget && !state.spent.is_empty() {
        return Sent::Stop;
    }
    *budget = budget.saturating_sub(cost);
    state.spent.push_back((now, cost));
    track.last_sent = Some(now);
    match live::set_live(session, &set) {
        Ok(_) => {
            track.delivered = delivered;
            track.unanswered = None;
            track.held = Some((sample, value.clone()));
            track.expired = false;
            state.rate_strikes = 0;
            *report.sent.entry(integration).or_default() += 1;
            Sent::Accepted
        }
        Err(LiveError::Refused {
            reason: LiveReason::Rate,
            retry_ms,
            ..
        }) => {
            track.unanswered = None;
            state.rate_strikes += 1;
            let wait = Duration::from_millis(u64::from(retry_ms.unwrap_or(1000).clamp(1, 1000)))
                * 2_u32.pow(state.rate_strikes.min(4) - 1);
            state.blocked_until = Some(now + wait.min(BACKOFF_MAX));
            Sent::Stop
        }
        Err(LiveError::Refused {
            reason:
                LiveReason::Layout
                | LiveReason::Page
                | LiveReason::Tile
                | LiveReason::Stream
                | LiveReason::Type,
            ..
        }) => {
            state.active = None;
            state.read_failed = false;
            Sent::Stop
        }
        Err(LiveError::Refused {
            reason: LiveReason::Asset,
            error,
            ..
        }) => {
            track.unanswered = None;
            report
                .errors
                .insert(integration, format!("{error} (asset)"));
            Sent::AssetRefused
        }
        Err(LiveError::Refused { reason, error, .. }) => {
            track.delivered = delivered;
            track.unanswered = None;
            report
                .errors
                .insert(integration, format!("{error} ({})", reason.name()));
            Sent::Refused
        }
        Err(LiveError::Link(error)) => {
            track.unanswered = Some((n, sample, value.clone()));
            if matches!(value, LiveValue::Image(Some(_))) {
                state.covered.insert(tile_key(binding));
            }
            report.errors.insert(integration, error.to_string());
            Sent::Stop
        }
    }
}

fn still_reported(controls: &Controls, binding: &Binding, sample: &LiveSample) -> bool {
    controls
        .get(&binding.source.integration)
        .is_none_or(|control| control.live().contains(sample))
}

fn cover_side(layout: &Layout, binding: &Binding, caps: &LiveCaps) -> Option<u16> {
    let [long, short] = if caps.screen[0] >= caps.screen[1] {
        caps.screen
    } else {
        [caps.screen[1], caps.screen[0]]
    };
    let [w, h] = match layout.orientation {
        Orientation::Landscape => [long, short],
        Orientation::Portrait => [short, long],
    };
    let screen = ScreenSize {
        w: u16::try_from(w).ok()?,
        h: u16::try_from(h).ok()?,
    };
    let rect = tile_rect(layout, &binding.page, &binding.tile, screen)?;
    let limit = u16::try_from(caps.image[0].min(caps.image[1])).unwrap_or(u16::MAX);
    let side = art_size(rect.w, rect.h)?.min(limit);
    (side >= COVER_SIDE_MIN).then_some(side)
}

fn tile_key(binding: &Binding) -> TileKey {
    (binding.page.clone(), binding.tile.clone())
}

fn key(binding: &Binding) -> (String, String, String) {
    (
        binding.page.clone(),
        binding.tile.clone(),
        binding.stream.clone(),
    )
}

/// Reads which Layout the Device shows: the built-in one, or the stored one, whose exact bytes name it.
fn read_active(session: &Session) -> Result<Active, String> {
    let state = session
        .layout_state(LAYOUT_STATE_TIMEOUT)
        .map_err(|error| error.to_string())?;
    if !matches!(state.source.as_str(), "builtin" | "storage") {
        return Err("Unknown Layout source".into());
    }
    let (reference, layout, slots) = if state.source == "builtin" {
        let slots = [Orientation::Landscape, Orientation::Portrait]
            .map(|orientation| (orientation, stored(session, orientation)))
            .to_vec();
        (live::BUILTIN.to_owned(), Layout::parse(BUILTIN), slots)
    } else {
        let orientation: Orientation = state.orientation;
        let report = xfer::get_layout(session, orientation).map_err(|error| error.to_string())?;
        let reference = live::layout_ref(&report.data);
        let slots = vec![(orientation, Some(reference.clone()))];
        (reference, Layout::parse(&report.data), slots)
    };
    let layout = layout.map_err(|errors| format!("{} Layout problems", errors.len()))?;
    if layout.orientation != state.orientation {
        return Err("Layout orientation mismatch".into());
    }
    Ok(Active {
        reference,
        bindings: bindings(&layout),
        layout: Arc::new(layout),
        slots,
    })
}

fn stored(session: &Session, orientation: Orientation) -> Option<String> {
    xfer::get_layout(session, orientation)
        .ok()
        .map(|report| live::layout_ref(&report.data))
}

fn slots_unchanged(session: &Session, active: &Active) -> bool {
    active
        .slots
        .iter()
        .all(|(orientation, reference)| stored(session, *orientation) == *reference)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;
    use td_core::layout::Tile;
    use td_link::{MockDeck, SessionConfig};

    use super::*;
    use crate::live::covers::CoverCache;
    use crate::live::fake::FakeControl;
    use crate::photos::encode::tests::noise_png;
    use td_host::integrations::Control;

    const GRAPH: &[u8] = include_bytes!("../../../../../schema/examples/graph.json");
    const CAPS: LiveCaps = LiveCaps {
        live_hz: 2,
        live_total_hz: 16,
        series: 32,
        text_bytes: 0,
        live_images: 0,
        image: [64, 64],
        screen: [1024, 600],
    };

    fn open(device: &MockDeck) -> Session {
        let session = Session::open(
            Box::new(device.connect()),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .unwrap()
        .0;
        assert_eq!(
            session.identity(Duration::from_secs(1)).unwrap().protocol,
            2
        );
        session
    }

    fn show(session: &Session, layout: &Layout) {
        xfer::put_layout(session, Orientation::Landscape, layout).unwrap();
    }

    fn key(integration: &str, stream: &str) -> StreamKey {
        StreamKey {
            integration: integration.into(),
            account: String::new(),
            stream: stream.into(),
        }
    }

    fn sample(sample: u64, value: LiveValue) -> LiveSample {
        LiveSample {
            account: String::new(),
            stream: String::new(),
            sample,
            ttl_ms: 5_000,
            value,
        }
    }

    fn numbers(entries: &[(&str, u64, f64)]) -> HashMap<StreamKey, LiveSample> {
        entries
            .iter()
            .map(|(stream, n, v)| (key("this_pc", stream), sample(*n, LiveValue::Number(*v))))
            .collect()
    }

    /// One page of graph Tiles, each bound to the `test` Integration stream it names.
    fn graphs(streams: &[(&str, StreamType)]) -> Layout {
        let mut layout = Layout::parse(GRAPH).unwrap();
        let Page::Grid {
            tiles, rows, cols, ..
        } = &mut layout.pages[0]
        else {
            unreachable!("graph.json starts with a grid page")
        };
        (*rows, *cols) = (8, 8);
        let template = tiles[0].clone();
        *tiles = streams
            .iter()
            .enumerate()
            .map(|(i, (field, kind))| Tile {
                id: format!("t{i}"),
                label: format!("T{i}"),
                row: (i / 8) as u8,
                col: (i % 8) as u8,
                streams: Some(BTreeMap::from([("value".to_owned(), *kind)])),
                action: Action::Widget {
                    source: "test".into(),
                    field: (*field).into(),
                    account: String::new(),
                },
                ..template.clone()
            })
            .collect();
        layout
    }

    fn tiles(device: &MockDeck) -> Vec<String> {
        device
            .live_sets()
            .iter()
            .map(|set| set["tile"].as_str().unwrap().to_owned())
            .collect()
    }

    fn ns(device: &MockDeck) -> Vec<u64> {
        device
            .live_sets()
            .iter()
            .map(|set| set["n"].as_u64().unwrap())
            .collect()
    }

    #[test]
    fn device_text_is_composed_folded_and_bounded() {
        let caps = LiveCaps {
            text_bytes: 6,
            ..CAPS
        };
        assert_eq!(
            caps.fit(&LiveValue::Text("  Ło\u{301}dź\nnext".into())),
            Some(LiveValue::Text("Lódz".into()))
        );
    }

    #[test]
    fn a_tile_binds_its_widget_actions_integration_stream() {
        let graph = Layout::parse(GRAPH).unwrap();
        assert_eq!(
            bindings(&graph),
            [Binding {
                page: "page".into(),
                tile: "cpu".into(),
                stream: "value".into(),
                kind: StreamType::Number,
                source: key("this_pc", "cpu_pct"),
            }]
        );
        let mut layout = graphs(&[("player", StreamType::Text)]);
        let Page::Grid { tiles, .. } = &mut layout.pages[0] else {
            unreachable!("graphs() builds a grid page")
        };
        tiles[0].streams = Some(BTreeMap::from([
            ("title".to_owned(), StreamType::Text),
            ("value".to_owned(), StreamType::Number),
        ]));
        let streams: Vec<_> = bindings(&layout)
            .into_iter()
            .map(|b| (b.stream, b.source.stream))
            .collect();
        assert_eq!(
            streams,
            [
                ("title".to_owned(), "title".to_owned()),
                ("value".to_owned(), "player".to_owned())
            ]
        );
    }

    #[test]
    fn only_bound_streams_and_only_new_samples_go() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        let first = numbers(&[
            ("cpu_pct", 1, 25.0),
            ("ram_pct", 1, 50.0),
            ("gpu_pct", 1, 5.0),
        ]);
        let report = router.deliver("d", 1, &session, CAPS, &first, t0);
        assert_eq!(report.sent, HashMap::from([("this_pc".to_owned(), 1)]));
        assert_eq!(router.wanted(), BTreeSet::from([key("this_pc", "cpu_pct")]));
        router.deliver("d", 1, &session, CAPS, &first, t0 + Duration::from_secs(1));
        assert_eq!(tiles(&device), ["cpu"]);
        let same_value = numbers(&[("cpu_pct", 2, 25.0), ("ram_pct", 2, 51.0)]);
        router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &same_value,
            t0 + Duration::from_secs(2),
        );
        assert_eq!(tiles(&device), ["cpu", "cpu"]);
        let cpu = device.live("page", "cpu", "value").unwrap();
        assert_eq!(cpu.samples, [25.0, 25.0]);
        assert_eq!(device.live_sets()[0]["ttl_ms"], 5_000);
    }

    #[test]
    fn a_stream_of_an_integration_turned_off_is_told_stale_once_and_fresh_again_when_it_returns() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        let flowing = numbers(&[("cpu_pct", 1, 25.0)]);
        router.deliver("d", 1, &session, CAPS, &flowing, t0);
        assert_eq!(device.live_sets().len(), 1);

        router.set_off(BTreeSet::from(["this_pc".to_owned()]));
        let gone = HashMap::new();
        let report = router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &gone,
            t0 + Duration::from_millis(100),
        );
        assert_eq!(
            report.sent,
            HashMap::new(),
            "the Device was just sent a value"
        );
        let report = router.deliver("d", 1, &session, CAPS, &gone, t0 + Duration::from_secs(1));
        assert_eq!(report.sent, HashMap::from([("this_pc".to_owned(), 1)]));
        let sets = device.live_sets();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[1]["ttl_ms"], 1_000);
        assert_eq!(sets[1]["value"].as_f64(), Some(25.0));
        std::thread::sleep(Duration::from_millis(1_100));
        assert!(device.live("page", "cpu", "value").unwrap().stale);

        for second in 2..6 {
            router.deliver(
                "d",
                1,
                &session,
                CAPS,
                &gone,
                t0 + Duration::from_secs(second),
            );
        }
        assert_eq!(
            device.live_sets().len(),
            2,
            "nothing more is sent while off"
        );

        router.set_off(BTreeSet::new());
        let back = numbers(&[("cpu_pct", 2, 40.0)]);
        router.deliver("d", 1, &session, CAPS, &back, t0 + Duration::from_secs(6));
        let shown = device.live("page", "cpu", "value").unwrap();
        assert_eq!(
            (shown.stale, shown.samples.last().copied()),
            (false, Some(40.0))
        );
    }

    #[test]
    fn a_stream_missing_while_its_integration_is_on_is_left_to_expire_on_its_own() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &numbers(&[("cpu_pct", 1, 25.0)]),
            t0,
        );
        for second in 1..4 {
            router.deliver(
                "d",
                1,
                &session,
                CAPS,
                &HashMap::new(),
                t0 + Duration::from_secs(second),
            );
        }
        assert_eq!(device.live_sets().len(), 1);
    }

    #[test]
    fn samples_between_rounds_coalesce_and_each_stream_waits_its_gap() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        router.deliver("d", 1, &session, CAPS, &numbers(&[("cpu_pct", 1, 1.0)]), t0);
        let early = numbers(&[("cpu_pct", 2, 2.0)]);
        router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &early,
            t0 + Duration::from_millis(300),
        );
        assert_eq!(device.live_sets().len(), 1);
        let latest = numbers(&[("cpu_pct", 3, 3.0)]);
        router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &latest,
            t0 + Duration::from_millis(900),
        );
        assert_eq!(
            device.live("page", "cpu", "value").unwrap().samples,
            [1.0, 3.0]
        );
    }

    #[test]
    fn a_rate_refusal_holds_the_device_until_its_window_ends() {
        let device = MockDeck::v2();
        let session = open(&device);
        let fields: Vec<String> = (0..20).map(|i| format!("s{i}")).collect();
        let streams: Vec<(&str, StreamType)> = fields
            .iter()
            .map(|f| (f.as_str(), StreamType::Number))
            .collect();
        show(&session, &graphs(&streams));
        let generous = LiveCaps {
            live_total_hz: 64,
            ..CAPS
        };
        let samples: HashMap<_, _> = fields
            .iter()
            .map(|f| (key("test", f), sample(1, LiveValue::Number(1.0))))
            .collect();
        let mut router = Router::default();
        let t0 = Instant::now();
        device.set_live_time(0);
        let report = router.deliver("d", 1, &session, generous, &samples, t0);
        assert_eq!(
            report.sent["test"],
            u64::from(td_link::mock::MOCK_LIVE_TOTAL_HZ)
        );
        let held = router.deliver(
            "d",
            1,
            &session,
            generous,
            &samples,
            t0 + Duration::from_millis(1),
        );
        assert!(held.sent.is_empty());
        device.set_live_time(1_050);
        let later = router.deliver(
            "d",
            1,
            &session,
            generous,
            &samples,
            t0 + Duration::from_millis(1_050),
        );
        assert_eq!(later.sent["test"], 4);
        assert_eq!(device.live_sets().len(), 20);
    }

    #[test]
    fn a_new_session_or_layout_sends_every_value_again_with_rising_n() {
        let device = MockDeck::v2();
        let session = open(&device);
        let graph = Layout::parse(GRAPH).unwrap();
        show(&session, &graph);
        let mut router = Router::default();
        let t0 = Instant::now();
        let samples = numbers(&[("cpu_pct", 1, 7.0)]);
        router.deliver("d", 1, &session, CAPS, &samples, t0);
        session.identity(Duration::from_secs(1)).unwrap();
        assert!(device.live("page", "cpu", "value").is_none());
        router.deliver(
            "d",
            2,
            &session,
            CAPS,
            &samples,
            t0 + Duration::from_secs(1),
        );
        assert_eq!(device.live("page", "cpu", "value").unwrap().samples, [7.0]);

        let mut renamed = graph.clone();
        renamed.name = "Graph 2".into();
        show(&session, &renamed);
        router.layout_changed("d");
        router.deliver(
            "d",
            2,
            &session,
            CAPS,
            &samples,
            t0 + Duration::from_secs(2),
        );
        assert_eq!(device.live("page", "cpu", "value").unwrap().samples, [7.0]);
        assert_eq!(device.live_sets()[2]["layout"], device.live_layout().0);

        renamed.name = "Graph 3".into();
        show(&session, &renamed);
        let newer = numbers(&[("cpu_pct", 2, 8.0)]);
        router.deliver("d", 2, &session, CAPS, &newer, t0 + Duration::from_secs(3));
        router.deliver("d", 2, &session, CAPS, &newer, t0 + Duration::from_secs(4));
        assert_eq!(device.live("page", "cpu", "value").unwrap().samples, [8.0]);
        let n = ns(&device);
        assert!(n.windows(2).all(|pair| pair[0] < pair[1]), "{n:?}");
    }

    #[test]
    fn an_unanswered_send_repeats_with_the_same_n() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        router.deliver("d", 1, &session, CAPS, &numbers(&[("cpu_pct", 1, 3.0)]), t0);
        device.delay_replies(Duration::from_millis(2_300));
        let samples = numbers(&[("cpu_pct", 2, 4.0)]);
        let lost = router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &samples,
            t0 + Duration::from_secs(1),
        );
        assert!(lost.errors.contains_key("this_pc"));
        device.delay_replies(Duration::ZERO);
        std::thread::sleep(Duration::from_millis(400));
        let retried = router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &samples,
            t0 + Duration::from_secs(2),
        );
        assert_eq!(retried.sent["this_pc"], 1);
        assert_eq!(
            device.live("page", "cpu", "value").unwrap().samples,
            [3.0, 4.0]
        );
        assert_eq!(ns(&device), [1, 2]);
        let newer = numbers(&[("cpu_pct", 3, 5.0)]);
        router.deliver("d", 1, &session, CAPS, &newer, t0 + Duration::from_secs(3));
        assert_eq!(ns(&device), [1, 2, 3]);
    }

    #[test]
    fn a_request_over_256_bytes_costs_more_of_the_device_budget() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(
            &session,
            &graphs(&[("a", StreamType::Series), ("b", StreamType::Series)]),
        );
        let long: Vec<f64> = (0..40).map(|i| f64::from(i) + 0.123_456_789).collect();
        let samples: HashMap<_, _> = ["a", "b"]
            .into_iter()
            .map(|f| (key("test", f), sample(1, LiveValue::Series(long.clone()))))
            .collect();
        let tight = LiveCaps {
            live_total_hz: 4,
            ..CAPS
        };
        let mut router = Router::default();
        let t0 = Instant::now();
        router.deliver("d", 1, &session, tight, &samples, t0);
        let sets = device.live_sets();
        assert_eq!(sets.len(), 1);
        let mut request = sets[0].as_object().unwrap().clone();
        request.remove("at_ms");
        let bytes = serde_json::to_vec(&Value::Object(request)).unwrap().len();
        assert_eq!(live::cost(bytes), 3, "{bytes} bytes");
        let tile = sets[0]["tile"].as_str().unwrap();
        let shown = device.live("page", tile, "value").unwrap();
        assert_eq!(shown.samples, long[8..]);
        router.deliver(
            "d",
            1,
            &session,
            tight,
            &samples,
            t0 + Duration::from_secs(1),
        );
        assert_eq!(tiles(&device).len(), 2);
    }

    #[test]
    fn a_value_the_tile_cannot_take_is_reported_and_never_sent() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        let huge = numbers(&[("cpu_pct", 1, 2e9)]);
        let report = router.deliver("d", 1, &session, CAPS, &huge, t0);
        assert!(report.errors["this_pc"].contains("number"));
        let text: HashMap<_, _> = [(
            key("this_pc", "cpu_pct"),
            sample(2, LiveValue::Text("hi".into())),
        )]
        .into();
        router.deliver("d", 1, &session, CAPS, &text, t0 + Duration::from_secs(1));
        assert!(device.live_sets().is_empty());
    }
    const MEDIA_CAPS: LiveCaps = LiveCaps {
        text_bytes: 128,
        live_images: 4,
        image: [256, 256],
        ..CAPS
    };

    fn control(id: &str, control: &Arc<FakeControl>) -> Controls {
        Arc::new(BTreeMap::from([(
            id.to_owned(),
            control.clone() as Arc<dyn td_host::integrations::Control>,
        )]))
    }

    fn media_layout(rows: u8, cols: u8) -> Layout {
        let mut value = serde_json::json!({
            "schema": 2, "name": "Media", "orientation": "landscape",
            "pages": [{"kind": "grid", "id": "main", "title": "Media", "rows": rows, "cols": cols, "tiles": [{
                "id": "player", "icon": null, "label": "Player", "row": 0, "col": 0, "rowSpan": 1, "colSpan": 1,
                "render": "now-playing",
                "streams": {"title": "text", "artist": "text", "cover": "image", "state": "number"},
                "action": {"type": "widget", "source": "media", "account": "", "field": "title"}
            }]}]
        });
        if rows > 1 {
            value["pages"][0]["tiles"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "id": "other", "icon": null, "label": "Other", "row": 1, "col": 0, "rowSpan": 1, "colSpan": 1,
                    "render": "now-playing",
                    "streams": {"title": "text", "artist": "text", "cover": "image", "state": "number"},
                    "action": {"type": "widget", "source": "media", "account": "", "field": "title"}
                }));
        }
        Layout::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn track(fake: &FakeControl, title: &str, cover: Option<&str>) {
        fake.publish(&[
            ("title", LiveValue::Text(title.into())),
            ("artist", LiveValue::Text(String::new())),
            (
                "state",
                LiveValue::Number(if title.is_empty() { 0.0 } else { 2.0 }),
            ),
            ("cover", LiveValue::Image(cover.map(str::to_owned))),
        ]);
    }

    fn read(fake: &FakeControl) -> HashMap<StreamKey, LiveSample> {
        fake.samples()
            .into_iter()
            .map(|s| (key("media", &s.stream), s))
            .collect()
    }

    fn make_covers(router: &mut Router, fake: &FakeControl, cache: &mut CoverCache) {
        let samples = read(fake);
        let covers = router
            .cover_jobs(&samples)
            .into_iter()
            .map(|job| {
                let LiveValue::Image(Some(source)) = &job.sample.value else {
                    unreachable!("a job names an image")
                };
                let cover = fake
                    .picture(source)
                    .and_then(|bytes| cache.prepare(source, &bytes, job.side).ok());
                ((job.source, job.sample.sample, job.side), cover)
            })
            .collect();
        router.set_covers(covers);
    }

    struct Media {
        deck: MockDeck,
        session: Session,
        fake: Arc<FakeControl>,
        router: Router,
        cache: CoverCache,
        start: Instant,
        now: Instant,
    }

    impl Media {
        fn new(rows: u8) -> Media {
            let fake = Arc::new(FakeControl::default());
            for seed in 1..=6 {
                fake.add_picture(&format!("sha256:art{seed}"), noise_png(300, 300, seed));
            }
            let deck = MockDeck::v2();
            deck.set_media_capabilities(true);
            let session = open(&deck);
            show(&session, &media_layout(rows, 1));
            let mut router = Router::default();
            router.set_controls(control("media", &fake));
            let now = Instant::now();
            Media {
                deck,
                session,
                fake,
                router,
                cache: CoverCache::default(),
                start: now,
                now,
            }
        }

        fn deliver(&mut self) -> HashMap<String, String> {
            let mut errors = self.round().errors;
            make_covers(&mut self.router, &self.fake, &mut self.cache);
            self.now += CONTROL_GAP;
            errors.extend(self.round().errors);
            self.now += CONTROL_GAP;
            errors
        }

        fn round(&mut self) -> Report {
            self.deck
                .set_live_time(self.now.duration_since(self.start).as_millis() as u64);
            let samples = read(&self.fake);
            self.router
                .deliver("d", 1, &self.session, MEDIA_CAPS, &samples, self.now)
        }

        fn shown(&self, stream: &str) -> Value {
            self.deck.live("main", "player", stream).unwrap().value
        }

        fn puts(&self) -> usize {
            self.deck
                .received()
                .iter()
                .filter(|r| r["m"] == "v2.asset.put")
                .count()
        }
    }

    #[test]
    fn a_cover_fills_the_tiles_art_box_and_goes_up_once() {
        let mut media = Media::new(1);
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        let Value::String(reference) = media.shown("cover") else {
            panic!("the cover is set");
        };
        let id = reference.strip_prefix("sha256:").unwrap();
        let jpeg = media.deck.assets()[id].clone();
        let picture = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((picture.width(), picture.height()), (256, 256));
        assert!(jpeg.len() <= 32 * 1024);
        assert_eq!(media.puts(), 1);
        track(&media.fake, "Second", Some("sha256:art2"));
        media.deliver();
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        assert_eq!(media.shown("cover"), Value::String(reference));
        assert_eq!(media.puts(), 2);
    }

    #[test]
    fn a_prepared_cover_uploads_during_the_display_gap() {
        let mut m = Media::new(1);
        let start = m.now;
        track(&m.fake, "First", Some("sha256:art1"));
        m.round();
        make_covers(&mut m.router, &m.fake, &mut m.cache);
        m.now = start + Duration::from_millis(100);
        assert!(m.round().errors.is_empty());
        assert_eq!(m.puts(), 1);
        assert!(m.shown("cover").is_null());
        m.now = start + Duration::from_millis(500);
        assert!(m.round().errors.is_empty());
        assert_eq!(m.puts(), 1);
        assert!(m.shown("cover").is_string());
        assert_eq!(m.router.devices["d"].rate_strikes, 0);
    }

    #[test]
    fn a_smaller_tile_gets_a_smaller_cover() {
        let mut media = Media::new(2);
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        let Value::String(reference) = media.shown("cover") else {
            panic!("the cover is set");
        };
        let jpeg = media.deck.assets()[reference.strip_prefix("sha256:").unwrap()].clone();
        let side = image::load_from_memory(&jpeg).unwrap().width();
        let rect =
            td_core::live::cell_rect(ScreenSize { w: 1024, h: 600 }, false, [2, 1], [0, 0, 1, 1])
                .unwrap();
        assert_eq!(Some(side as u16), art_size(rect.w, rect.h));
        assert!(side < 256);
        assert_eq!(
            media.deck.live("main", "other", "cover").unwrap().value,
            Value::String(reference)
        );
        assert_eq!(media.puts(), 1);
    }

    #[test]
    fn a_new_title_clears_the_old_cover_before_it_shows() {
        let mut media = Media::new(1);
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        assert!(media.shown("cover").is_string());
        let start = media.deck.live_sets().len();
        media
            .fake
            .publish(&[("title", LiveValue::Text("Second".into()))]);
        media.round();
        let sets = media.deck.live_sets()[start..].to_vec();
        let clear = sets
            .iter()
            .position(|v| v["stream"] == "cover" && v["value"].is_null())
            .expect("the old cover is cleared");
        let title = sets
            .iter()
            .position(|v| v["stream"] == "title" && v["value"] == "Second")
            .expect("the new title goes");
        assert!(clear < title);
    }

    #[test]
    fn idle_media_sends_all_four_streams() {
        let mut media = Media::new(1);
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        track(&media.fake, "", None);
        media.deliver();
        assert_eq!(media.shown("state"), serde_json::json!(0));
        assert_eq!(media.shown("title"), serde_json::json!(""));
        assert_eq!(media.shown("artist"), serde_json::json!(""));
        assert!(media.shown("cover").is_null());
    }

    #[test]
    fn a_cover_the_device_lost_goes_up_again() {
        let mut media = Media::new(1);
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        let Value::String(reference) = media.shown("cover") else {
            panic!("the cover is set");
        };
        track(&media.fake, "Second", None);
        media.deliver();
        assert!(media.shown("cover").is_null());
        let id = reference.strip_prefix("sha256:").unwrap();
        td_link::asset::delete_asset(&media.session, id).unwrap();
        track(&media.fake, "First", Some("sha256:art1"));
        let report = media.round();
        make_covers(&mut media.router, &media.fake, &mut media.cache);
        media.now += CONTROL_GAP;
        let report = [report, media.round()];
        assert!(
            report
                .iter()
                .any(|r| r.errors.get("media").is_some_and(|e| e.contains("asset")))
        );
        assert!(media.shown("cover").is_null());
        media.now += CONTROL_GAP;
        media.round();
        assert_eq!(media.shown("cover"), Value::String(reference));
        assert_eq!(media.puts(), 2);
    }

    #[test]
    fn a_device_keeps_at_most_four_covers_and_says_when_it_is_full() {
        let mut media = Media::new(1);
        for seed in 1..=6 {
            track(
                &media.fake,
                &format!("Track {seed}"),
                Some(&format!("sha256:art{seed}")),
            );
            media.deliver();
            assert!(media.shown("cover").is_string());
            assert!(media.deck.assets().len() <= 4);
        }
        media.deck.set_asset_capacity(0);
        track(&media.fake, "Full", Some("sha256:art1"));
        let errors = media.deliver();
        assert_eq!(errors.get("media").map(String::as_str), Some(OUT_OF_SPACE));
        assert!(media.shown("cover").is_null());
        assert!(media.deck.assets().is_empty());
    }

    #[test]
    fn pruning_protects_current_covers_and_forgotten_covers_can_be_uploaded_again() {
        let mut media = Media::new(1);
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        let first = media
            .shown("cover")
            .as_str()
            .unwrap()
            .strip_prefix("sha256:")
            .unwrap()
            .to_owned();
        assert_eq!(
            media.router.cover_assets("d"),
            BTreeSet::from([first.clone()])
        );
        track(&media.fake, "Second", Some("sha256:art2"));
        media.deliver();
        assert!(!media.router.cover_assets("d").contains(&first));
        td_link::asset::delete_asset(&media.session, &first).unwrap();
        media.router.forget_cover("d", &first);
        let puts = media.puts();
        track(&media.fake, "First", Some("sha256:art1"));
        media.deliver();
        assert_eq!(media.puts(), puts + 1);
        assert_eq!(media.shown("cover"), format!("sha256:{first}"));
    }

    #[test]
    fn a_session_starts_by_sweeping_leftover_covers() {
        let mut cache = CoverCache::default();
        let cover = cache.prepare("left", &noise_png(60, 60, 9), 64).unwrap();
        let mut media = Media::new(1);
        td_link::asset::put_asset(
            &media.session,
            &cover.bytes,
            td_link::asset::AssetFormat::Jpeg,
        )
        .unwrap();
        media.round();
        assert!(media.deck.assets().is_empty());
    }

    #[test]
    fn a_sample_the_control_replaced_is_not_sent() {
        let mut media = Media::new(1);
        track(&media.fake, "First", None);
        let stale = read(&media.fake);
        track(&media.fake, "Second", None);
        media
            .router
            .deliver("d", 1, &media.session, MEDIA_CAPS, &stale, media.now);
        assert!(media.deck.live("main", "player", "title").is_none());
        media.round();
        assert_eq!(media.shown("title"), serde_json::json!("Second"));
    }

    #[test]
    fn a_cover_made_for_an_older_sample_is_never_used() {
        let mut media = Media::new(1);
        track(&media.fake, "First", Some("sha256:art1"));
        media.round();
        make_covers(&mut media.router, &media.fake, &mut media.cache);
        media
            .fake
            .publish(&[("cover", LiveValue::Image(Some("sha256:art2".into())))]);
        media.now += CONTROL_GAP;
        media.round();
        assert!(media.shown("cover").is_null());
        assert_eq!(media.puts(), 0);
    }

    #[test]
    fn control_streams_are_paced_at_half_a_second() {
        let deck = MockDeck::v2();
        let session = open(&deck);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let fake = Arc::new(FakeControl::default());
        let mut router = Router::default();
        router.set_controls(control("this_pc", &fake));
        let now = Instant::now();
        for ms in [0_u32, 499, 500] {
            fake.publish(&[("cpu_pct", LiveValue::Number(f64::from(ms)))]);
            let samples = fake
                .samples()
                .into_iter()
                .map(|s| (key("this_pc", &s.stream), s))
                .collect();
            router.deliver(
                "d",
                1,
                &session,
                CAPS,
                &samples,
                now + Duration::from_millis(u64::from(ms)),
            );
        }
        assert_eq!(
            deck.live("page", "cpu", "value").unwrap().samples,
            [0.0, 500.0]
        );
    }
    #[test]
    fn review_track_change_waits_for_cover_gap() {
        let mut m = Media::new(1);
        let start = m.now;
        track(&m.fake, "First", Some("sha256:art1"));
        m.round();
        make_covers(&mut m.router, &m.fake, &mut m.cache);
        m.now = start + Duration::from_millis(500);
        m.round();
        assert!(m.shown("cover").is_string());
        track(&m.fake, "Second", None);
        m.fake
            .publish(&[("artist", LiveValue::Text("New artist".into()))]);
        m.now = start + Duration::from_millis(600);
        let count = m.deck.live_sets().len();
        m.round();
        assert_eq!(
            m.deck.live_sets().len(),
            count,
            "metadata and clear must wait"
        );
        assert_eq!(m.shown("title"), "First");
        m.now = start + Duration::from_millis(1000);
        m.round();
        assert!(m.shown("cover").is_null());
        assert_eq!(m.shown("title"), "Second");
        assert_eq!(m.shown("artist"), "New artist");
        assert_eq!(m.router.devices["d"].rate_strikes, 0);
    }
    #[test]
    fn review_disabled_media_streams_are_not_fitted() {
        let caps = LiveCaps {
            text_bytes: 0,
            live_images: 0,
            ..MEDIA_CAPS
        };
        assert!(caps.fit(&LiveValue::Text("hello".into())).is_none());
        assert!(caps.fit(&LiveValue::Image(None)).is_none());
    }

    #[test]
    fn review_busy_uploads_back_off_during_frequent_wakes() {
        let mut m = Media::new(1);
        track(&m.fake, "First", Some("sha256:art1"));
        m.round();
        make_covers(&mut m.router, &m.fake, &mut m.cache);
        m.deck.refuse_assets("v2.asset.put", "busy", 3);
        let start = m.now;
        for ms in (500..=2500).step_by(50) {
            m.now = start + Duration::from_millis(ms);
            m.round();
        }
        assert!(m.puts() <= 3, "put attempts: {}", m.puts());
        m.now = start + Duration::from_millis(4500);
        m.round();
        assert!(m.shown("cover").is_string());
    }

    #[test]
    fn review_unanswered_cover_is_cleared_before_next_title() {
        let mut m = Media::new(1);
        track(&m.fake, "First", Some("sha256:art1"));
        m.deliver();
        m.fake.publish(&[("cover", LiveValue::Image(None))]);
        m.round();
        m.now += CONTROL_GAP;
        m.fake
            .publish(&[("cover", LiveValue::Image(Some("sha256:art1".into())))]);
        make_covers(&mut m.router, &m.fake, &mut m.cache);
        m.deck.delay_replies(Duration::from_millis(2300));
        assert!(m.round().errors.contains_key("media"));
        m.deck.delay_replies(Duration::ZERO);
        std::thread::sleep(Duration::from_millis(400));
        assert!(m.shown("cover").is_string());
        m.fake
            .publish(&[("title", LiveValue::Text("Second".into()))]);
        let start = m.deck.live_sets().len();
        m.now += CONTROL_GAP;
        m.round();
        let sets = m.deck.live_sets()[start..].to_vec();
        let clear = sets
            .iter()
            .position(|s| s["stream"] == "cover" && s["value"].is_null())
            .expect("clear the possibly delivered cover");
        let title = sets.iter().position(|s| s["stream"] == "title").unwrap();
        assert!(clear < title);
    }

    #[test]
    fn sweep_errors_back_off_without_blocking_metadata() {
        let mut m = Media::new(1);
        m.deck.refuse_assets("v2.asset.list", "busy", 3);
        track(&m.fake, "First", None);
        let start = m.now;
        for ms in (0..=2000).step_by(50) {
            m.now = start + Duration::from_millis(ms);
            m.round();
        }
        assert_eq!(m.shown("title"), "First");
        assert_eq!(
            m.deck
                .received()
                .iter()
                .filter(|r| r["m"] == "v2.asset.list")
                .count(),
            3
        );
        m.now = start + Duration::from_millis(4000);
        m.round();
        assert!(m.router.devices["d"].swept);
    }
    #[test]
    fn four_showing_covers_defer_a_cover_only_replacement() {
        let deck = MockDeck::v2();
        deck.set_media_capabilities(true);
        deck.set_live_time(0);
        let session = open(&deck);
        let mut layout = serde_json::to_value(media_layout(1, 1)).unwrap();
        let tile = layout["pages"][0]["tiles"][0].clone();
        layout["pages"][0]["rows"] = serde_json::json!(4);
        layout["pages"][0]["tiles"] = serde_json::json!(
            (0..4)
                .map(|n| {
                    let mut t = tile.clone();
                    t["id"] = serde_json::json!(format!("player{n}"));
                    t["row"] = serde_json::json!(n);
                    t["action"]["source"] = serde_json::json!(format!("media{n}"));
                    t
                })
                .collect::<Vec<_>>()
        );
        let layout = Layout::parse(&serde_json::to_vec(&layout).unwrap()).unwrap();
        show(&session, &layout);
        let mut router = Router::default();
        let start = Instant::now();
        let mut samples = HashMap::new();
        for n in 0..4 {
            samples.insert(
                key(&format!("media{n}"), "title"),
                sample(1, LiveValue::Text("First".into())),
            );
            samples.insert(
                key(&format!("media{n}"), "cover"),
                sample(1, LiveValue::Image(Some(format!("art{n}")))),
            );
        }
        router.deliver("d", 1, &session, MEDIA_CAPS, &samples, start);
        let mut cache = CoverCache::default();
        let prepared = router
            .cover_jobs(&samples)
            .into_iter()
            .enumerate()
            .map(|(n, j)| {
                let cover = cache
                    .prepare(&format!("art{n}"), &noise_png(64, 64, n as u32), j.side)
                    .unwrap();
                ((j.source, j.sample.sample, j.side), Some(cover))
            })
            .collect();
        router.set_covers(prepared);
        deck.set_live_time(1000);
        router.deliver(
            "d",
            1,
            &session,
            MEDIA_CAPS,
            &samples,
            start + Duration::from_secs(1),
        );
        assert_eq!(deck.assets().len(), 4);
        for n in 0..4 {
            assert!(
                deck.live("main", &format!("player{n}"), "cover")
                    .unwrap()
                    .value
                    .is_string()
            );
        }
        samples.insert(
            key("media0", "cover"),
            sample(2, LiveValue::Image(Some("new".into()))),
        );
        let job = router
            .cover_jobs(&samples)
            .into_iter()
            .find(|j| j.source.integration == "media0")
            .unwrap();
        let cover = cache
            .prepare("new", &noise_png(64, 64, 10), job.side)
            .unwrap();
        router
            .covers
            .insert((job.source, 2, job.side), Some(cover.clone()));
        deck.set_live_time(2000);
        router.deliver(
            "d",
            1,
            &session,
            MEDIA_CAPS,
            &samples,
            start + Duration::from_secs(2),
        );
        assert_eq!(deck.assets().len(), 4);
        assert!(
            deck.live("main", "player0", "cover")
                .unwrap()
                .value
                .is_null()
        );
        assert_eq!(
            deck.live("main", "player0", "title").unwrap().value,
            "First"
        );
        deck.set_live_time(3000);
        router.deliver(
            "d",
            1,
            &session,
            MEDIA_CAPS,
            &samples,
            start + Duration::from_secs(3),
        );
        assert_eq!(deck.assets().len(), 4);
        assert_eq!(
            deck.live("main", "player0", "cover").unwrap().value,
            format!("sha256:{}", cover.id)
        );
    }

    fn layout_gets(device: &MockDeck) -> usize {
        device
            .received()
            .iter()
            .filter(|request| request["m"] == "layout.get")
            .count()
    }

    #[test]
    fn a_layout_changed_behind_the_routers_back_is_read_again_without_samples() {
        let device = MockDeck::v2();
        let session = open(&device);
        let graph = Layout::parse(GRAPH).unwrap();
        show(&session, &graph);
        let mut router = Router::default();
        let t0 = Instant::now();
        let none = HashMap::new();
        router.deliver("d", 1, &session, CAPS, &none, t0);
        assert_eq!(router.shown()["d"].1.name, graph.name);
        let mut renamed = graph.clone();
        renamed.name = "Graph 2".into();
        show(&session, &renamed);
        let gets = layout_gets(&device);
        router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &none,
            t0 + Duration::from_millis(4_900),
        );
        assert_eq!(router.shown()["d"].1.name, graph.name);
        assert_eq!(layout_gets(&device), gets);
        router.deliver("d", 1, &session, CAPS, &none, t0 + Duration::from_secs(5));
        assert_eq!(router.shown()["d"].1.name, "Graph 2");
    }

    #[test]
    fn checking_an_unchanged_layout_costs_one_layout_get_and_sends_nothing_again() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        let samples = numbers(&[("cpu_pct", 1, 7.0)]);
        router.deliver("d", 1, &session, CAPS, &samples, t0);
        assert_eq!(device.live_sets().len(), 1);
        let (gets, lines) = (layout_gets(&device), device.lines().len());
        router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &samples,
            t0 + Duration::from_secs(5),
        );
        assert_eq!(layout_gets(&device), gets + 1);
        assert_eq!(device.lines().len(), lines);
        assert_eq!(device.live_sets().len(), 1);
    }

    #[test]
    fn a_device_on_the_builtin_layout_is_checked_without_a_console_line() {
        let device = MockDeck::v2();
        let session = open(&device);
        let mut router = Router::default();
        let t0 = Instant::now();
        let none = HashMap::new();
        router.deliver("d", 1, &session, CAPS, &none, t0);
        let builtin = Layout::parse(BUILTIN).unwrap();
        assert_eq!(router.shown()["d"].1.name, builtin.name);
        let lines = device.lines().len();
        router.deliver("d", 1, &session, CAPS, &none, t0 + Duration::from_secs(5));
        assert_eq!(device.lines().len(), lines);
        show(&session, &Layout::parse(GRAPH).unwrap());
        router.deliver("d", 1, &session, CAPS, &none, t0 + Duration::from_secs(10));
        assert_eq!(router.shown()["d"].1.name, "Graph");
    }

    #[test]
    fn a_new_layout_ends_the_wait_after_a_failed_read() {
        let device = MockDeck::v2();
        let session = open(&device);
        show(&session, &Layout::parse(GRAPH).unwrap());
        let mut router = Router::default();
        let t0 = Instant::now();
        let samples = numbers(&[("cpu_pct", 1, 7.0)]);
        device.lose_console_replies(1);
        router.deliver("d", 1, &session, CAPS, &samples, t0);
        assert!(router.read_failed("d", 1));
        // Fail the state query before a transfer opens, keeping this session usable.
        // Clear its stale console waiter; the following ping fences the console reply.
        session.send_line("layout state").unwrap();
        session
            .call(&serde_json::json!({"m":"ping"}), Duration::from_secs(1))
            .unwrap();
        router.layout_changed("d");
        router.deliver(
            "d",
            1,
            &session,
            CAPS,
            &samples,
            t0 + Duration::from_secs(1),
        );
        assert!(!router.read_failed("d", 1));
        assert_eq!(device.live("page", "cpu", "value").unwrap().samples, [7.0]);
    }

    #[test]
    fn a_new_layout_keeps_a_rate_backoff_the_device_asked_for() {
        let device = MockDeck::v2();
        let session = open(&device);
        let fields: Vec<String> = (0..20).map(|i| format!("s{i}")).collect();
        let streams: Vec<(&str, StreamType)> = fields
            .iter()
            .map(|f| (f.as_str(), StreamType::Number))
            .collect();
        show(&session, &graphs(&streams));
        let generous = LiveCaps {
            live_total_hz: 64,
            ..CAPS
        };
        let samples: HashMap<_, _> = fields
            .iter()
            .map(|f| (key("test", f), sample(1, LiveValue::Number(1.0))))
            .collect();
        let mut router = Router::default();
        let t0 = Instant::now();
        device.set_live_time(0);
        router.deliver("d", 1, &session, generous, &samples, t0);
        let lines = device.lines().len();
        router.layout_changed("d");
        let held = router.deliver(
            "d",
            1,
            &session,
            generous,
            &samples,
            t0 + Duration::from_millis(1),
        );
        assert!(held.sent.is_empty());
        assert_eq!(device.lines().len(), lines);
    }
}
