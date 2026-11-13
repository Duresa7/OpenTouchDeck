//! The Device manager. It watches the USB ports, probes each Espressif port with `hello` and
//! sorts it into a Device, a New board or a busy port. Each connected Device gets an actor
//! thread that owns its link session. Blocking `td-link` work never runs on the async runtime:
//! probes run on the scan thread and requests on each Device's actor thread.
mod actor;
mod error;
mod name;
mod operations;
mod pairings_session;
mod settings;
mod settings_session;
pub use error::DeviceError;
pub use name::{NAME_BYTES, NameProblem, name_problem};
pub use operations::Operations;
pub use pairings::{
    DevicePairing, DevicePairings, DevicePairingsError, PairingPhase, PairingSnapshot,
};
pub use pairings_session::{Forgetting, PairingsSession, PairingsView, offers_pairings, over_usb};
pub use settings::{
    DeviceSettings, DeviceSettingsPatch, PhotoFramePatch, PhotoFrameSettings, SettingsField,
    SettingsOutcome, SettingsPatched,
};
pub use settings_session::{
    Field, Load, Needs, Notice, NoticeKind, RESTART_TIMEOUT, Restart, START_AFTER_S,
    SettingsSession, SettingsView, duration_label, orientation_name, orientation_of,
    start_after_choices,
};
pub mod console;
mod pairings;
mod ports;
mod scan;
mod store;
mod wifi;

#[cfg(any(test, feature = "test-hooks"))]
pub use ports::{BoardKind, MockPorts};
pub use ports::{PortInfo, PortSource, RealPorts};
pub use wifi::{
    Browse, KeychainError, OsKeychain, PairError, PairingKeys, PairingStarted, WifiConfig,
    WifiDevice, WifiPairing,
};
#[cfg(any(test, feature = "test-hooks"))]
pub use wifi::{MemoryKeys, MockNetwork};

use crate::api::{Event, Events, Topic};
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use td_link::Session;
use ts_rs::TS;

/// A Device or New board id: the USB serial number, lowercased with separators removed. It
/// matches `^[0-9a-z]{1,32}$`, so it's safe in a path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
pub struct DeviceId(String);
impl DeviceId {
    pub fn parse(id: &str) -> Option<Self> {
        let valid = !id.is_empty()
            && id.len() <= 32
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        valid.then(|| Self(id.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let id = String::deserialize(d)?;
        Self::parse(&id).ok_or_else(|| {
            serde::de::Error::custom("a Device id is 1 to 32 lowercase letters and digits")
        })
    }
}
impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::ops::Deref for DeviceId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl PartialEq<&str> for DeviceId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Which Firmware build a Device runs, and so which bundle its updates come from. The radio build is the opt-in build
/// with Wi-Fi and Bluetooth; the normal build has no radio code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum FirmwareChannel {
    #[default]
    Normal,
    Radio,
}
impl FirmwareChannel {
    pub fn of(radio: bool) -> Self {
        if radio { Self::Radio } else { Self::Normal }
    }
    pub fn is_radio(self) -> bool {
        self == Self::Radio
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct FirmwareInfo {
    pub version: String,
    pub protocol: u32,
    pub build: String,
    pub recovery: bool,
    pub update_available: Option<String>,
    /// The build the Device says it runs. Firmware that doesn't say runs the normal build.
    #[serde(default)]
    pub channel: FirmwareChannel,
}
/// What a v2 `hello` says the Device can show and store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Capabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub anim: Option<AnimationCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gif_frames: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gif_pixels: Option<u32>,
    /// The radio build's Bluetooth keyboard: the most computers and phones it keeps bonded, 1 to 8. Absent when the
    /// Firmware has no Bluetooth keyboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ble_hid: Option<u32>,
    pub screen: [u32; 2],
    pub grid: [u32; 2],
    pub pages: u32,
    pub tiles: u32,
    pub layout_bytes: u32,
    pub layout_schemas: Vec<u32>,
    pub storage_free: u32,
    pub tile_kinds: Vec<String>,
    pub asset_bytes: u32,
    pub formats: Vec<String>,
    pub image: [u32; 2],
    pub photo_bytes: u32,
    /// Live values: updates a stream and the whole Device take each second, samples a graph keeps, and bytes a
    /// text value holds. All zero when the Firmware has no Live tiles.
    pub live_hz: u32,
    pub live_total_hz: u32,
    pub series: u32,
    pub text_bytes: u32,
    /// The most Tiles in a Layout that declare an `image` stream. Zero when the Firmware draws no cover art.
    pub live_images: u32,
    /// `true` when the Device can join a Wi-Fi network (the radio build). Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub wifi: Option<bool>,
}
impl From<&td_link::Capabilities> for Capabilities {
    fn from(c: &td_link::Capabilities) -> Self {
        Self {
            anim: c.anim.as_ref().map(|a| AnimationCapabilities {
                image: a.image,
                min_delay_ms: a.min_delay_ms,
                page_tiles: a.page_tiles,
                page_pixels: a.page_pixels,
            }),
            gif_frames: Some(c.gif_frames),
            gif_pixels: Some(c.gif_pixels),
            ble_hid: c.ble_hid.filter(|bonds| *bonds > 0),
            screen: c.screen,
            grid: c.grid,
            pages: c.pages,
            tiles: c.tiles,
            layout_bytes: c.layout_bytes,
            layout_schemas: c.layout_schemas.clone(),
            storage_free: c.free_bytes,
            tile_kinds: c.tile_kinds.clone(),
            asset_bytes: c.asset_bytes,
            formats: c.formats.clone(),
            image: c.image,
            photo_bytes: c.photo_bytes,
            live_hz: c.live_hz,
            live_total_hz: c.live_total_hz,
            series: c.series,
            text_bytes: c.text_bytes,
            live_images: c.live_images.unwrap_or(0),
            wifi: c.wifi.filter(|wifi| *wifi),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum LinkState {
    Connected {
        port: String,
        since_ms: u64,
        protocol: u32,
    },
    Connecting {
        port: String,
    },
    /// Known, not plugged in.
    Offline,
    /// The link dropped on its own. The Companion keeps retrying.
    Lost {
        reason: LostReason,
        detail: String,
        at_ms: u64,
    },
    /// The user chose Disconnect. No auto-connect until replug or Connect.
    Disconnected,
    /// Another program holds the port.
    Busy {
        port: String,
        detail: String,
    },
    /// A known serial that doesn't answer `hello`.
    Unresponsive {
        port: String,
    },
    /// A Firmware Job owns the port.
    Updating {
        job: String,
    },
}
/// How the Companion reaches a connected Device. USB wins when both are there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Carrier {
    Usb,
    Wifi,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum LostReason {
    Unplugged,
    Timeout,
    IoError,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum SetupState {
    NeedsSetup,
    Ready,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct KeyMirrorState {
    pub enabled: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceDiagnostics {
    pub product: String,
    pub running_from: String,
    pub sinks: Vec<String>,
    pub current_layout: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ControlLayout {
    Unavailable {
        reason: ControlLayoutUnavailable,
    },
    NoPushedCopy {
        orientation: td_core::Orientation,
        has_controls: bool,
    },
    Matches {
        orientation: td_core::Orientation,
        has_controls: bool,
    },
    Differs {
        orientation: td_core::Orientation,
        has_controls: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ControlLayoutUnavailable {
    Offline,
    Checking,
    Unsupported,
    ReadFailed,
}
impl Default for ControlLayout {
    fn default() -> Self {
        Self::Unavailable {
            reason: ControlLayoutUnavailable::Checking,
        }
    }
}
/// What a Device shows: the Layout this Companion last read from it and the Page on its screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LayoutSummary {
    pub source: LayoutSource,
    pub name: String,
    pub orientation: td_core::Orientation,
    pub pages: u32,
    /// The index of the Page on screen. Unknown until the Device reports a Page of this Layout, and always on Firmware
    /// that doesn't report Pages.
    pub page: Option<u32>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutSource {
    /// The Firmware's built-in Layout, because no Layout is stored for its orientation.
    Builtin,
    Storage,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShownLayout {
    source: LayoutSource,
    name: String,
    orientation: td_core::Orientation,
    pages: Vec<String>,
}
impl ShownLayout {
    pub(crate) fn new(layout: &td_core::Layout, source: LayoutSource) -> Self {
        Self {
            source,
            name: layout.name.clone(),
            orientation: layout.orientation,
            pages: layout.pages.iter().map(|p| p.id().to_owned()).collect(),
        }
    }
    fn summary(&self, page: Option<&str>) -> LayoutSummary {
        LayoutSummary {
            source: self.source,
            name: self.name.clone(),
            orientation: self.orientation,
            pages: self.pages.len() as u32,
            page: page
                .and_then(|page| self.pages.iter().position(|id| id == page))
                .map(|index| index as u32),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceSummary {
    pub id: DeviceId,
    pub rev: u64,
    pub name: Option<String>,
    pub setup: SetupState,
    /// While connected, `port` is the serial port over USB and `host:port` over Wi-Fi.
    pub link: LinkState,
    /// The Carrier of the link, `null` unless connected.
    pub carrier: Option<Carrier>,
    pub firmware: Option<FirmwareInfo>,
    pub diagnostics: Option<DeviceDiagnostics>,
    pub capabilities: Option<Capabilities>,
    /// The orientation the Device shows, from the Layout it shows. Null whenever `layout` is.
    pub orientation: Option<td_core::Orientation>,
    pub layout: Option<LayoutSummary>,
    pub control_layout: ControlLayout,
    pub key_mirror: KeyMirrorState,
    pub stable_id: bool,
}
/// A board with the Espressif USB ids that doesn't answer as Open TouchDeck Firmware.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NewBoard {
    pub id: DeviceId,
    pub port: String,
    #[serde(flatten)]
    pub probe: BoardProbe,
    pub seen_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "probe", rename_all = "kebab-case")]
pub enum BoardProbe {
    Silent,
    Busy { detail: String },
    OtherFirmware { product: String, detail: String },
}
/// One port's last probe, for troubleshooting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PortProbe {
    pub port: String,
    pub usb_serial: Option<String>,
    pub id: DeviceId,
    #[serde(flatten)]
    pub status: PortStatus,
    pub probed_ms: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum PortStatus {
    /// Not probed yet, or waiting to retry.
    Pending,
    Device {
        protocol: u32,
        version: String,
    },
    Silent,
    Busy {
        detail: String,
    },
    OtherFirmware {
        product: String,
        detail: String,
    },
}

#[derive(Clone, Serialize, Deserialize, TS)]
pub struct DeviceAdded(pub DeviceSummary);
impl Event for DeviceAdded {
    const NAME: &'static str = "device.added";
    fn device(&self) -> Option<&str> {
        Some(self.0.id.as_str())
    }
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct DeviceUpdated(pub DeviceSummary);
impl Event for DeviceUpdated {
    const NAME: &'static str = "device.updated";
    fn device(&self) -> Option<&str> {
        Some(self.0.id.as_str())
    }
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct DeviceRemoved {
    pub id: DeviceId,
}
impl Event for DeviceRemoved {
    const NAME: &'static str = "device.removed";
    fn device(&self) -> Option<&str> {
        Some(self.id.as_str())
    }
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct NewBoardAdded(pub NewBoard);
impl Event for NewBoardAdded {
    const NAME: &'static str = "new-board.added";
    fn device(&self) -> Option<&str> {
        Some(self.0.id.as_str())
    }
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct NewBoardRemoved {
    pub id: DeviceId,
}
impl Event for NewBoardRemoved {
    const NAME: &'static str = "new-board.removed";
    fn device(&self) -> Option<&str> {
        Some(self.id.as_str())
    }
}
/// One console line from a Device.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct DeviceConsole {
    pub device: DeviceId,
    pub line: String,
}
impl Event for DeviceConsole {
    const NAME: &'static str = "device.console";
    const TOPIC: Topic = Topic::Console;
    fn device(&self) -> Option<&str> {
        Some(self.device.as_str())
    }
}
/// One mirrored key report and what the PC did with it.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct DeviceKeyMirror {
    pub device: DeviceId,
    pub seq: u32,
    pub report: String,
    /// `typed`, `duplicate`, `off`, `unknown` or `suppressed`.
    pub outcome: String,
    /// The sequence numbers that never arrived, whose held keys were released.
    pub gap: Option<[u32; 2]>,
    pub errors: Vec<String>,
    pub arrived_ms: u64,
}
impl Event for DeviceKeyMirror {
    const NAME: &'static str = "device.key-mirror";
    const TOPIC: Topic = Topic::KeyMirror;
    fn device(&self) -> Option<&str> {
        Some(self.device.as_str())
    }
}
/// A raw Device event, for diagnostics.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct DeviceEvent {
    pub device: DeviceId,
    pub event: serde_json::Value,
}
impl Event for DeviceEvent {
    const NAME: &'static str = "device.event";
    const TOPIC: Topic = Topic::DeviceEvents;
    fn device(&self) -> Option<&str> {
        Some(self.device.as_str())
    }
}

pub(crate) type ActionHandler = Arc<dyn Fn(&DeviceId, &serde_json::Value) + Send + Sync>;
/// Makes the sink a Device's key mirror types into.
pub type InputFactory = Arc<dyn Fn() -> Result<Box<dyn td_host::InputSink>, String> + Send + Sync>;
/// How the manager reaches ports, PC input and storage. Tests swap in mocks.
pub struct Config {
    pub source: Arc<dyn PortSource>,
    pub input: InputFactory,
    /// `devices.json`. `None` keeps known Devices in memory only.
    pub store: Option<PathBuf>,
    /// Pairings, their keys and discovery.
    pub wifi: WifiConfig,
    #[cfg(any(test, feature = "test-hooks"))]
    pub mock: Option<(MockPorts, td_host::FakeSink)>,
}
impl Config {
    /// The real ports, the PC's input, the OS keychain, mDNS, and `devices.json` and
    /// `network.json` under `root`.
    pub fn real(root: &std::path::Path) -> Self {
        Self {
            source: Arc::new(RealPorts::default()),
            input: Arc::new(td_host::input::system_sink),
            store: Some(root.join("devices.json")),
            wifi: WifiConfig::real(root),
            #[cfg(any(test, feature = "test-hooks"))]
            mock: None,
        }
    }
    /// Mock ports and a fake input sink that records what it would type.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn mock(store: Option<PathBuf>) -> Self {
        let ports = MockPorts::default();
        let sink = td_host::FakeSink::default();
        let factory = sink.clone();
        let network = store.as_ref().map(|p| p.with_file_name("network.json"));
        Self {
            source: Arc::new(ports.clone()),
            input: Arc::new(move || Ok(Box::new(factory.clone()) as Box<dyn td_host::InputSink>)),
            store,
            wifi: WifiConfig::mock(network),
            mock: Some((ports, sink)),
        }
    }
}

/// Why a request can't reach a Device's link.
#[derive(Debug)]
pub enum Reach {
    NotFound,
    NotConnected(LinkState),
}
#[derive(Debug)]
pub enum Forget {
    NotFound,
    Connected,
}

struct Entry {
    summary: DeviceSummary,
    actor: Option<actor::Handle>,
    /// Off after Disconnect, until replug or Connect.
    auto: bool,
    quick_drops: u32,
    shown: Option<ShownLayout>,
    reported_page: Option<String>,
}
struct PortRecord {
    info: PortInfo,
    id: DeviceId,
    stable: bool,
    status: PortStatus,
    probed_ms: Option<u64>,
    appeared: Instant,
    due: Instant,
    backoff: Duration,
}
#[derive(Default)]
struct Registry {
    devices: BTreeMap<DeviceId, Entry>,
    boards: BTreeMap<DeviceId, NewBoard>,
    ports: BTreeMap<String, PortRecord>,
    /// Paired Devices the Wi-Fi thread tries while they have no USB port.
    wifi: BTreeMap<DeviceId, wifi::WifiRecord>,
    generation: u64,
    updating: std::collections::BTreeSet<DeviceId>,
    installed: std::collections::BTreeSet<DeviceId>,
}
/// Wakes the scan thread early: a USB change, a rescan, a Connect or a lost link.
#[derive(Default)]
struct Wake {
    kicked: Mutex<bool>,
    cv: Condvar,
}
impl Wake {
    fn kick(&self) {
        *self.kicked.lock() = true;
        self.cv.notify_all();
    }
    fn wait(&self, timeout: Duration) {
        let mut kicked = self.kicked.lock();
        if !*kicked {
            self.cv.wait_for(&mut kicked, timeout);
        }
        *kicked = false;
    }
}
struct Inner {
    events: Events,
    reg: Mutex<Registry>,
    wake: Wake,
    stop: AtomicBool,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
    dialogs: AtomicUsize,
    input_gate: Mutex<()>,
    config: OnceLock<Config>,
    actions: OnceLock<ActionHandler>,
    controls: OnceLock<std::sync::Weak<crate::controls::DeviceControls>>,
    wifi: wifi::WifiState,
    pairing_progress: tokio::sync::watch::Sender<BTreeMap<DeviceId, PairingSnapshot>>,
}
impl Inner {
    fn track(&self, thread: std::thread::JoinHandle<()>) {
        let mut threads = self.threads.lock();
        threads.retain(|thread| !thread.is_finished());
        threads.push(thread);
    }
    fn config(&self) -> &Config {
        self.config.get().expect("the Device manager has started")
    }
    /// Applies `change` to a Device's summary and publishes it if anything changed.
    fn update(&self, reg: &mut Registry, id: &DeviceId, change: impl FnOnce(&mut DeviceSummary)) {
        let Some(entry) = reg.devices.get_mut(id) else {
            return;
        };
        let mut next = entry.summary.clone();
        change(&mut next);
        if next.link != entry.summary.link {
            self.pairing_progress.send_if_modified(|all| {
                if let Some(state) = all.get_mut(id) {
                    state.stale = true;
                    true
                } else {
                    false
                }
            });
            next.control_layout = ControlLayout::default();
            entry.shown = None;
            entry.reported_page = None;
        }
        if !matches!(next.link, LinkState::Connected { .. }) {
            next.carrier = None;
        }
        next.layout = entry
            .shown
            .as_ref()
            .map(|shown| shown.summary(entry.reported_page.as_deref()));
        next.orientation = next.layout.as_ref().map(|layout| layout.orientation);
        if next == entry.summary {
            return;
        }
        next.rev = entry.summary.rev + 1;
        entry.summary = next;
        self.events.publish(&DeviceUpdated(entry.summary.clone()));
    }
    fn page_shown(&self, id: &DeviceId, generation: u64, page: &str) {
        let mut reg = self.reg.lock();
        let Some(entry) = reg.devices.get_mut(id) else {
            return;
        };
        if entry.actor.as_ref().map(|a| a.connection) != Some(generation) {
            return;
        }
        entry.reported_page = Some(page.to_owned());
        self.update(&mut reg, id, |_| {});
    }
    fn set_link(&self, reg: &mut Registry, id: &DeviceId, link: LinkState) {
        self.update(reg, id, |d| d.link = link);
    }
    /// Writes `devices.json`. Called off the async runtime.
    fn save(&self, reg: &Registry) {
        let Some(path) = &self.config().store else {
            return;
        };
        let saved: Vec<_> = reg
            .devices
            .values()
            .map(|e| store::Saved {
                id: e.summary.id.clone(),
                name: e.summary.name.clone(),
                setup: e.summary.setup,
            })
            .collect();
        if let Err(error) = store::save(path, &saved) {
            eprintln!("Companion: couldn't save {}: {error}", path.display());
        }
    }
    /// True while a dialog holds input, and for good once the Devices stop.
    fn input_suppressed(&self) -> bool {
        self.dialogs.load(Ordering::SeqCst) > 0 || self.stop.load(Ordering::SeqCst)
    }
}

/// Finds and runs every Device. Cloning shares the same manager.
#[derive(Clone)]
pub struct DeviceManager {
    inner: Arc<Inner>,
}
impl DeviceManager {
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_capability_fixture(&self, id: &DeviceId, capabilities: Option<Capabilities>) {
        let mut reg = self.inner.reg.lock();
        self.inner
            .update(&mut reg, id, |d| d.capabilities = capabilities);
    }
    #[cfg(test)]
    pub(crate) fn pause_key_reports(&self) -> parking_lot::MutexGuard<'_, ()> {
        self.inner.input_gate.lock()
    }

    pub(crate) fn action_handler(&self, handler: ActionHandler) {
        let _ = self.inner.actions.set(handler);
    }
    pub(crate) fn controls(&self, handler: &Arc<crate::controls::DeviceControls>) {
        let _ = self.inner.controls.set(Arc::downgrade(handler));
    }
    pub fn new(events: Events) -> Self {
        Self {
            inner: Arc::new(Inner {
                events,
                reg: Mutex::new(Registry::default()),
                wake: Wake::default(),
                stop: AtomicBool::new(false),
                threads: Mutex::new(Vec::new()),
                dialogs: AtomicUsize::new(0),
                input_gate: Mutex::new(()),
                config: OnceLock::new(),
                actions: OnceLock::new(),
                controls: OnceLock::new(),
                wifi: wifi::WifiState::default(),
                pairing_progress: tokio::sync::watch::channel(BTreeMap::new()).0,
            }),
        }
    }
    /// Loads the known Devices and starts the scan thread. Later calls do nothing.
    pub fn start(&self, config: Config) {
        if self.inner.config.set(config).is_err() {
            return;
        }
        if let Some(path) = &self.inner.config().store {
            let mut reg = self.inner.reg.lock();
            for saved in store::load(path) {
                let summary = DeviceSummary {
                    id: saved.id.clone(),
                    rev: 1,
                    name: saved.name,
                    setup: saved.setup,
                    link: LinkState::Offline,
                    carrier: None,
                    firmware: None,
                    diagnostics: None,
                    capabilities: None,
                    orientation: None,
                    layout: None,
                    control_layout: ControlLayout::default(),
                    key_mirror: KeyMirrorState {
                        enabled: false,
                        reason: None,
                    },
                    stable_id: true,
                };
                reg.devices.insert(
                    saved.id,
                    Entry {
                        summary,
                        actor: None,
                        auto: true,
                        quick_drops: 0,
                        shown: None,
                        reported_page: None,
                    },
                );
            }
        }
        self.inner.load_wifi(&mut self.inner.reg.lock());
        let inner = self.inner.clone();
        self.inner.track(
            std::thread::Builder::new()
                .name("td-device-scan".into())
                .spawn(move || scan::run(inner))
                .expect("the scan thread starts"),
        );
        let inner = self.inner.clone();
        self.inner.track(
            std::thread::Builder::new()
                .name("td-device-wifi".into())
                .spawn(move || wifi::run(inner))
                .expect("the Wi-Fi thread starts"),
        );
    }
    /// Stops the scan and Wi-Fi threads, which then close every link.
    /// From here on, key reports type nothing; `wait_stopped` lets one already typing finish.
    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        self.inner.wake.kick();
        self.inner.wifi.wake.kick();
    }
    /// Blocking: after `stop`, waits up to `limit` for discovery, actors and forwarders, including
    /// key release. Returns false if a thread was still running at the deadline, for example one
    /// stuck in a USB write the kernel won't finish; that thread is left to end with the process.
    pub(crate) fn wait_stopped(&self, limit: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + limit;
        // A key report that began before `stop` may still be typing; reports after it are suppressed.
        match self.inner.input_gate.try_lock_until(deadline) {
            Some(gate) => drop(gate),
            None => return false,
        }
        loop {
            // Discovery may register actor threads while its final probe is completing.
            let threads = std::mem::take(&mut *self.inner.threads.lock());
            if threads.is_empty() {
                return true;
            }
            for thread in threads {
                while !thread.is_finished() {
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                let _ = thread.join();
            }
        }
    }
    pub(crate) fn control_layout(&self, device: &str, session: Option<u64>, status: ControlLayout) {
        let Some(id) = DeviceId::parse(device) else {
            return;
        };
        let mut reg = self.inner.reg.lock();
        self.inner.update(&mut reg, &id, |d| {
            let current = match d.link {
                LinkState::Connected { since_ms, .. } => Some(since_ms),
                _ => None,
            };
            if current == session {
                d.control_layout = status;
            }
        });
    }
    pub(crate) fn layout_shown(&self, device: &str, session: u64, shown: ShownLayout) {
        let Some(id) = DeviceId::parse(device) else {
            return;
        };
        let mut reg = self.inner.reg.lock();
        let Some(entry) = reg.devices.get_mut(&id) else {
            return;
        };
        if !matches!(entry.summary.link, LinkState::Connected { since_ms, .. } if since_ms == session)
        {
            return;
        }
        entry.shown = Some(shown);
        self.inner.update(&mut reg, &id, |_| {});
    }
    pub fn summaries(&self) -> Vec<DeviceSummary> {
        let reg = self.inner.reg.lock();
        reg.devices.values().map(|e| e.summary.clone()).collect()
    }
    pub fn get(&self, id: &DeviceId) -> Option<DeviceSummary> {
        let reg = self.inner.reg.lock();
        reg.devices.get(id).map(|e| e.summary.clone())
    }
    pub fn new_boards(&self) -> Vec<NewBoard> {
        self.inner.reg.lock().boards.values().cloned().collect()
    }
    /// The last probe of every Espressif port. Reading it never opens a port.
    pub fn ports(&self) -> Vec<PortProbe> {
        let reg = self.inner.reg.lock();
        reg.ports
            .values()
            .map(|r| PortProbe {
                port: r.info.port.clone(),
                usb_serial: r.info.usb_serial.clone(),
                id: r.id.clone(),
                status: r.status.clone(),
                probed_ms: r.probed_ms,
            })
            .collect()
    }
    /// Probes every port that isn't linked again now.
    pub fn rescan(&self) {
        let mut reg = self.inner.reg.lock();
        let now = Instant::now();
        for record in reg.ports.values_mut() {
            record.due = now;
            record.backoff = scan::FIRST_RETRY;
        }
        drop(reg);
        self.inner.wake.kick();
    }
    /// Turns auto-connect back on after a Disconnect.
    pub fn connect(&self, id: &DeviceId) -> Option<DeviceSummary> {
        let mut reg = self.inner.reg.lock();
        if reg.updating.contains(id) {
            return reg.devices.get(id).map(|e| e.summary.clone());
        }
        let entry = reg.devices.get_mut(id)?;
        entry.auto = true;
        let linked = entry.actor.is_some();
        let port = reg.ports.values_mut().find(|r| &r.id == id);
        if !linked {
            let link = match port {
                Some(record) => {
                    record.due = Instant::now();
                    record.backoff = scan::FIRST_RETRY;
                    LinkState::Connecting {
                        port: record.info.port.clone(),
                    }
                }
                None => match reg.wifi.get(id) {
                    Some(record) => LinkState::Connecting {
                        port: record.address.to_string(),
                    },
                    None => LinkState::Offline,
                },
            };
            self.inner.set_link(&mut reg, id, link);
            self.inner.wifi_due_now(&mut reg, id);
        }
        let summary = reg.devices[id].summary.clone();
        drop(reg);
        self.inner.wake.kick();
        Some(summary)
    }
    /// Closes the link and keeps the Device down until replug or Connect.
    pub fn disconnect(&self, id: &DeviceId) -> Option<DeviceSummary> {
        let mut reg = self.inner.reg.lock();
        if reg.updating.contains(id) {
            return reg.devices.get(id).map(|e| e.summary.clone());
        }
        let entry = reg.devices.get_mut(id)?;
        entry.auto = false;
        // Dropping the handle ends the actor thread, which closes the session and the port.
        let actor = entry.actor.take();
        self.inner.set_link(&mut reg, id, LinkState::Disconnected);
        let summary = reg.devices[id].summary.clone();
        drop(reg);
        drop(actor);
        Some(summary)
    }
    /// Forgets a Device that isn't connected. Blocking: it writes `devices.json`.
    pub fn forget(&self, id: &DeviceId) -> Result<(), Forget> {
        let mut reg = self.inner.reg.lock();
        if reg.updating.contains(id) {
            return Err(Forget::Connected);
        }
        let entry = reg.devices.get(id).ok_or(Forget::NotFound)?;
        if entry.actor.is_some() {
            return Err(Forget::Connected);
        }
        reg.devices.remove(id);
        self.inner.pairing_progress.send_modify(|all| {
            all.remove(id);
        });
        self.inner.events.publish(&DeviceRemoved { id: id.clone() });
        self.inner.save(&reg);
        let paired = reg.wifi.contains_key(id);
        drop(reg);
        // A forgotten Device keeps no Pairing either.
        if paired && let Err(error) = self.wifi_forget(id) {
            eprintln!("Companion: couldn't forget the Pairing with {id}: {error}");
        }
        Ok(())
    }
    /// Runs `work` on the Device's actor thread with its session.
    pub async fn call<T: Send + 'static>(
        &self,
        id: &DeviceId,
        work: impl FnOnce(&Session) -> T + Send + 'static,
    ) -> Result<T, Reach> {
        let not_connected = || {
            self.get(id)
                .map_or(Reach::NotFound, |d| Reach::NotConnected(d.link))
        };
        let sender = {
            let reg = self.inner.reg.lock();
            let entry = reg.devices.get(id).ok_or(Reach::NotFound)?;
            if reg.updating.contains(id) {
                return Err(Reach::NotConnected(entry.summary.link.clone()));
            }
            match &entry.actor {
                Some(actor) => actor.sender(),
                None => return Err(Reach::NotConnected(entry.summary.link.clone())),
            }
        };
        let (reply, answer) = tokio::sync::oneshot::channel();
        let job: actor::Work = Box::new(move |session| {
            let _ = reply.send(work(session));
        });
        sender.send(job).map_err(|_| not_connected())?;
        answer.await.map_err(|_| not_connected())
    }
    /// The connected Device's live session, for blocking work off the async runtime that
    /// runs alongside the actor's. Requests on one session are matched by sequence number.
    pub fn session(&self, id: &DeviceId) -> Result<Arc<Session>, Reach> {
        let reg = self.inner.reg.lock();
        let entry = reg.devices.get(id).ok_or(Reach::NotFound)?;
        match (&entry.actor, &entry.summary.link) {
            (Some(actor), LinkState::Connected { .. }) => Ok(actor.session()),
            (_, link) => Err(Reach::NotConnected(link.clone())),
        }
    }
    /// Restarts the Device and closes its link, so it comes back with a fresh `hello`: a
    /// restarted Device counts the app as offline until it gets one.
    pub async fn restart(&self, id: &DeviceId) -> Result<Result<(), td_link::LinkError>, Reach> {
        if let Err(error) = self.call(id, |s| s.send_line("reboot")).await? {
            return Ok(Err(error));
        }
        let mut reg = self.inner.reg.lock();
        let actor = reg.devices.get_mut(id).and_then(|e| e.actor.take());
        let now = Instant::now();
        let port = reg.ports.values_mut().find(|r| &r.id == id).map(|record| {
            record.status = PortStatus::Pending;
            record.appeared = now;
            record.due = now + scan::FIRST_RETRY;
            record.backoff = scan::FIRST_RETRY;
            record.info.port.clone()
        });
        if actor.is_some()
            && let Some(port) = port
        {
            self.inner
                .set_link(&mut reg, id, LinkState::Connecting { port });
        }
        drop(reg);
        drop(actor);
        self.inner.wake.kick();
        Ok(Ok(()))
    }
    /// Records a name the Device accepted.
    pub(crate) fn named(&self, id: &DeviceId, name: String) {
        let mut reg = self.inner.reg.lock();
        self.inner.update(&mut reg, id, |d| {
            d.name = Some(name);
            d.setup = SetupState::Ready;
        });
        self.inner.save(&reg);
    }
    /// Reserves a target against scanning and actor requests until Firmware work ends.
    pub fn firmware_hold(&self, id: &DeviceId, job: &str) -> Result<FirmwareHold, Reach> {
        let mut reg = self.inner.reg.lock();
        if !reg.devices.contains_key(id) && !reg.boards.contains_key(id) {
            return Err(Reach::NotFound);
        }
        let session = reg
            .devices
            .get(id)
            .and_then(|e| e.actor.as_ref())
            .map(|a| a.session());
        let port = reg
            .ports
            .values()
            .find(|r| &r.id == id)
            .map(|r| r.info.port.clone())
            .ok_or(Reach::NotFound)?;
        reg.updating.insert(id.clone());
        self.inner
            .set_link(&mut reg, id, LinkState::Updating { job: job.into() });
        Ok(FirmwareHold {
            manager: self.clone(),
            id: id.clone(),
            port,
            session,
        })
    }
    /// Suppresses key input from every Device while the guard lives. Approval and Firmware
    /// dialogs hold one, so Device input can't answer them.
    pub fn suppress_input(&self) -> InputSuppressed {
        let _gate = self.inner.input_gate.lock();
        self.inner.dialogs.fetch_add(1, Ordering::SeqCst);
        InputSuppressed(self.inner.clone())
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn mock(&self) -> Option<&(MockPorts, td_host::FakeSink)> {
        self.inner.config.get()?.mock.as_ref()
    }
}
/// Key input stays suppressed until every guard drops.
pub struct InputSuppressed(Arc<Inner>);
impl Drop for InputSuppressed {
    fn drop(&mut self) {
        self.0.dialogs.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The Device manager owns the handoff and resumes discovery on every exit path.
pub struct FirmwareHold {
    manager: DeviceManager,
    id: DeviceId,
    pub port: String,
    pub session: Option<Arc<Session>>,
}
struct FirmwareConnector {
    source: Arc<dyn PortSource>,
    port: String,
}
impl td_link::update::Connector for FirmwareConnector {
    fn connect(&mut self) -> Result<(Session, td_link::IncomingReceiver), String> {
        Session::open(
            self.source.open(&self.port).map_err(|e| e.to_string())?,
            td_link::SessionConfig {
                ping_every: None,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())
    }
}
impl FirmwareHold {
    pub fn connector(&self) -> Box<dyn td_link::update::Connector> {
        Box::new(FirmwareConnector {
            source: self.manager.inner.config().source.clone(),
            port: self.port.clone(),
        })
    }

    pub fn installed(&self) {
        self.manager
            .inner
            .reg
            .lock()
            .installed
            .insert(self.id.clone());
    }
}
impl Drop for FirmwareHold {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            session.close();
        }
        let mut reg = self.manager.inner.reg.lock();
        if let Some(entry) = reg.devices.get_mut(&self.id) {
            entry.actor.take();
            entry.auto = true;
        }
        reg.updating.remove(&self.id);
        drop(reg);
        self.manager.rescan();
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AnimationCapabilities {
    pub image: [u32; 2],
    pub min_delay_ms: u32,
    pub page_tiles: u32,
    pub page_pixels: u32,
}

#[cfg(test)]
mod wait_stopped_tests {
    use super::*;

    #[test]
    fn a_thread_stuck_in_io_cannot_hold_shutdown_forever() {
        let devices = DeviceManager::new(Events::default());
        // Stands in for a Device thread blocked in a serial flush that never returns.
        let (_release, stuck) = std::sync::mpsc::channel::<()>();
        devices.inner.track(std::thread::spawn(move || {
            let _ = stuck.recv();
        }));
        devices.stop();
        let started = std::time::Instant::now();
        assert!(!devices.wait_stopped(std::time::Duration::from_millis(100)));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn a_key_report_stuck_in_the_input_sink_cannot_hold_shutdown() {
        let devices = DeviceManager::new(Events::default());
        // Stands in for a key report stuck inside a synchronous input sink send.
        let stuck = devices.pause_key_reports();
        let (done, finished) = std::sync::mpsc::channel();
        let stopper = devices.clone();
        std::thread::spawn(move || {
            stopper.stop();
            let _ = done.send(());
        });
        assert!(
            finished
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok(),
            "stop must not wait for an in-flight key report"
        );
        let started = std::time::Instant::now();
        assert!(!devices.wait_stopped(std::time::Duration::from_millis(100)));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        drop(stuck);
    }
}

#[cfg(test)]
mod operations_tests;
#[cfg(test)]
mod sessions_tests;
