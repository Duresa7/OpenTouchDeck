//! A deck that speaks protocol 1 in process, so the app and its tests run without a board.
//! It keeps the state the firmware keeps and records every message it receives.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::{self, ErrorKind, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use td_core::frame::CHUNK_DATA_MAX;
use td_core::layout::{Page, StreamType, TileRender, UPLOAD_MAX};
use td_core::{
    Body, Event, Frame, Hello, ICON_PAGE, InputPhase, Item, Layout, Orientation, Role, Settings,
    StreamDecoder,
};

use crate::asset::{ASSET_PAGE, AssetFormat, asset_id, refusal, valid_asset_id};
use crate::live::{TTL_MAX_MS, TTL_MIN_MS, VALUE_MAX, layout_ref};
use crate::mock_ble::{MockBle, MockBond};
use crate::photo::{IDLE_INTERVAL_S, IDLE_TIMEOUT_S, IdleSettings, PHOTO_BYTES_MAX};
use crate::wifi::{WifiPassword, valid_password, valid_ssid};
use crate::xfer::{ICON_BYTES, LayoutState, RETRY_AFTER, TRANSMISSIONS, WINDOW, crc32};
use crate::{READ_TICK, Transport};

pub const MOCK_PORT: &str = "mock";
pub const MOCK_ELF_SHA256: &str =
    "6d6f636b6465636b6d6f636b6465636b6d6f636b6465636b6d6f636b6465636b";
const DASHBOARD_ROWS: usize = 8;
const ONLINE_FOR: Duration = Duration::from_secs(5);
const HEARTBEAT: Duration = Duration::from_secs(2);
const MOCK_PAGE_TILES: u64 = 4;
const MOCK_PAGE_PIXELS: u64 = 65536;
const BUILTIN: &[u8] = include_bytes!("../../../../schema/examples/builtin.json");
/// The space a mock Device has for Assets until a test sets another.
pub const ASSET_CAPACITY: u64 = 4 * 1024 * 1024;
const MEDIA_ASSET_BYTES: usize = 32768;
const GIF_ASSET_BYTES: usize = 262144;
const MEDIA_IMAGE: u16 = 256;
const MEDIA_TEXT_BYTES: usize = 128;
const MEDIA_LIVE_IMAGES: usize = 4;
/// The live caps a mock Device advertises, as the Firmware does.
pub const MOCK_LIVE_HZ: u32 = 2;
pub const MOCK_LIVE_TOTAL_HZ: u32 = 16;
pub const MOCK_SERIES: usize = 32;

/// Where a put lands once `xfer.end` accepts it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Layout(Orientation),
    Icon(String),
    Asset(String, AssetFormat),
    Photo(String),
    Update,
}

/// A put the deck is receiving into `<path>.tmp`.
#[derive(Debug)]
struct Inbound {
    xfer: u8,
    target: Target,
    bytes: usize,
    crc32: u32,
    data: Vec<u8>,
    next: u16,
}

/// A get the deck is sending, with the send time and count of each chunk in flight.
#[derive(Debug)]
struct Outbound {
    xfer: u8,
    data: Vec<u8>,
    base: u16,
    ahead: u16,
    sent: HashMap<u16, (Instant, u32)>,
}

impl Outbound {
    fn count(&self) -> u16 {
        self.data.len().div_ceil(CHUNK_DATA_MAX) as u16
    }

    fn chunk(&self, idx: u16) -> Vec<u8> {
        let start = usize::from(idx) * CHUNK_DATA_MAX;
        self.data[start..(start + CHUNK_DATA_MAX).min(self.data.len())].to_vec()
    }
}

/// One live stream as the Firmware keeps it. `sent` is the ttl, type and value of the accepted `n`, which an
/// identical retry must repeat.
#[derive(Debug, Default)]
struct LiveSlot {
    n: u32,
    sent: Value,
    samples: Vec<f64>,
    value: Value,
    expires: Option<Instant>,
    window: u64,
    in_window: u32,
}

/// The live values of the Layout on screen. The rate windows run from the last protocol selection.
#[derive(Debug)]
struct Live {
    session: Instant,
    slots: HashMap<(String, String, String), LiveSlot>,
    window: u64,
    in_window: u32,
}

impl Live {
    fn new() -> Live {
        Live {
            session: Instant::now(),
            slots: HashMap::new(),
            window: 0,
            in_window: 0,
        }
    }
}

/// What a mock Device shows for one live stream: the accepted `n`, the graph's samples oldest first, the last
/// value as sent, and whether it is drawn stale.
#[derive(Debug, Clone, PartialEq)]
pub struct MockLive {
    pub n: u32,
    pub samples: Vec<f64>,
    pub value: Value,
    pub stale: bool,
}

#[derive(Debug)]
struct State {
    hello: Hello,
    v2: bool,
    selected_v2: bool,
    device_name: String,
    gif_capabilities: bool,
    media_capabilities: bool,
    /// The radio build's Bluetooth keyboard.
    ble: MockBle,
    /// Runs the radio build: a v2 hello says `"radio": true`.
    radio: bool,
    settings: Settings,
    icons: BTreeSet<String>,
    icon_files: HashMap<String, Vec<u8>>,
    widgets: BTreeMap<String, Value>,
    dashboards: BTreeMap<String, Value>,
    sets: Vec<Value>,
    received: Vec<Value>,
    lines: Vec<String>,
    silent: bool,
    reply_delay: Duration,
    lose_console_replies: u32,
    frames_rx: u64,
    frames_tx: u64,
    dropped: u64,
    last_rx: Option<Instant>,
    online_since_hello: bool,
    refuse_sessions: bool,
    booted: Instant,
    link: Option<Sender<Vec<u8>>>,
    /// The link is a network carrier's, not USB's.
    link_network: bool,
    generation: u64,
    layouts: HashMap<Orientation, Vec<u8>>,
    shown: Orientation,
    last_xfer: u8,
    last_seq: u8,
    inbound: Option<Inbound>,
    outbound: Option<Outbound>,
    lose_in: Vec<u16>,
    lose_out: Vec<u16>,
    corrupt_next_put: bool,
    reboots: u32,
    key_seq: u32,
    press_seq: u32,
    input_seq: u32,
    update_sha: Option<String>,
    /// The update that applies at the next connect: the image's elf_sha256, its slot, and whether it is a radio build.
    update_pending: Option<(String, String, bool)>,
    restart: Option<(Duration, Duration)>,
    restart_window: Option<(Instant, Instant)>,
    update_mode: String,
    update_aborted: bool,
    assets: BTreeMap<String, Vec<u8>>,
    asset_capacity: u64,
    pinned_assets: BTreeSet<String>,
    chunk_bytes: usize,
    /// The xfer of the last Asset transfer that closed and what `v2.asset.end` answered for it.
    asset_end: Option<(u8, Map<String, Value>)>,
    live: Live,
    live_sets: Vec<Value>,
    live_time: Option<u64>,
    asset_refusals: VecDeque<(String, String)>,
    idle_settings: IdleSettings,
    idle: bool,
    photo_interval: Option<Duration>,
    last_photo: Option<Instant>,
    photos: Vec<Vec<u8>>,
    photo_refusals: Vec<(String, Option<u64>)>,
    photo_end: Option<(u8, Map<String, Value>)>,
    clock: Option<(u64, i32)>,
    page: usize,
    page_reports: bool,
    selections: u64,
    page_sent: Option<(u64, String, usize)>,
    wifi: MockWifi,
}

/// The radio build's Wi-Fi station, as far as the link shows it.
#[derive(Debug, Default)]
struct MockWifi {
    /// The Device answers `wifi.*` and advertises `caps.wifi`.
    capability: bool,
    /// Every `wifi.*` request arrives as if on a network Carrier.
    usb_only: bool,
    /// The networks in range, by SSID, with their passwords. Every one is made up.
    networks: BTreeMap<String, String>,
    /// The stored network, as NVS keeps it.
    stored: Option<(String, String)>,
    /// A set's join has not finished: the next `wifi.state` resolves it.
    joining: bool,
    /// `connected`, or `failed` with its reason, once the join resolved.
    outcome: Option<Result<(), &'static str>>,
}

impl MockWifi {
    fn fields(&self) -> Map<String, Value> {
        let (state, reason, ip, rssi) = match (&self.stored, self.joining, self.outcome) {
            (None, ..) => ("off", None, None, None),
            (Some(_), true, _) | (Some(_), false, None) => ("connecting", None, None, None),
            (Some(_), false, Some(Ok(()))) => ("connected", None, Some("192.0.2.10"), Some(-52)),
            (Some(_), false, Some(Err(reason))) => ("failed", Some(reason), None, None),
        };
        Map::from_iter([
            ("configured".to_owned(), json!(self.stored.is_some())),
            (
                "ssid".to_owned(),
                json!(self.stored.as_ref().map_or("", |(ssid, _)| ssid.as_str())),
            ),
            ("state".to_owned(), json!(state)),
            ("reason".to_owned(), json!(reason)),
            ("ip".to_owned(), json!(ip)),
            ("rssi".to_owned(), json!(rssi)),
        ])
    }

    /// Finishes a join the way the station would: the network in range with the right password joins.
    fn resolve(&mut self) {
        if !self.joining {
            return;
        }
        self.joining = false;
        self.outcome = self
            .stored
            .as_ref()
            .map(|(ssid, password)| match self.networks.get(ssid) {
                None => Err("not-found"),
                Some(expected) if expected != password => Err("wrong-password"),
                Some(_) => Ok(()),
            });
    }
}

fn arrived(request: &Map<String, Value>) -> Value {
    let mut set = request.clone();
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    set.insert("at_ms".into(), json!(at));
    Value::Object(set)
}

impl State {
    fn capabilities(&self) -> Value {
        let mut caps = json!({"screen":[1024,600],"grid":[8,8],"pages":8,"tiles":64,
            "layout_bytes":32768,"layout_schemas":[1,2],"free_bytes":asset_space(self).3,"tile_kinds":["static","graph","photo-frame","clock"],
            "asset_bytes":ICON_BYTES,"formats":["tdi1"],"image":[64,64],"gif_frames":0,"gif_pixels":0,
            "photo_bytes":PHOTO_BYTES_MAX,"live_hz":MOCK_LIVE_HZ,"live_total_hz":MOCK_LIVE_TOTAL_HZ,"series":MOCK_SERIES,"text_bytes":0,"streams":1});
        if self.gif_capabilities {
            caps["anim"] = json!({"image":[128,128],"min_delay_ms":100,"page_tiles":MOCK_PAGE_TILES,"page_pixels":MOCK_PAGE_PIXELS});
            caps["formats"] = json!(["tdi1", "gif"]);
            caps["tile_kinds"] = json!(["static", "graph", "gif", "photo-frame", "clock"]);
            caps["asset_bytes"] = json!(GIF_ASSET_BYTES);
            caps["gif_frames"] = json!(60);
            caps["gif_pixels"] = json!(983040);
        }
        if let Some(bonds) = self.ble.bonds_max {
            caps["ble_hid"] = json!(bonds);
        }
        if self.media_capabilities {
            let kinds = caps["tile_kinds"]
                .as_array_mut()
                .expect("tile kinds are a list");
            let at = kinds.len() - 2;
            kinds.splice(at..at, [json!("bar"), json!("now-playing")]);
            caps["formats"]
                .as_array_mut()
                .expect("formats are a list")
                .push(json!("jpeg"));
            caps["image"] = json!([MEDIA_IMAGE, MEDIA_IMAGE]);
            caps["asset_bytes"] = json!(self.asset_bytes());
            caps["text_bytes"] = json!(MEDIA_TEXT_BYTES);
            caps["streams"] = json!(4);
            caps["live_images"] = json!(MEDIA_LIVE_IMAGES);
        }
        if self.wifi.capability {
            caps["wifi"] = json!(true);
        }
        caps
    }

    fn asset_bytes(&self) -> usize {
        match (self.gif_capabilities, self.media_capabilities) {
            (true, _) => GIF_ASSET_BYTES,
            (false, true) => MEDIA_ASSET_BYTES,
            (false, false) => ICON_BYTES,
        }
    }

    fn shows(&self, id: &str) -> bool {
        let reference = format!("sha256:{id}");
        self.live
            .slots
            .values()
            .any(|slot| slot.value == reference.as_str())
    }

    fn record_set(&mut self, request: &Map<String, Value>) {
        self.sets.push(arrived(request));
    }

    fn record_live(&mut self, request: &Map<String, Value>) {
        self.live_sets.push(arrived(request));
    }
}

#[derive(Debug, Clone)]
pub struct MockDeck {
    state: Arc<Mutex<State>>,
}

impl Default for MockDeck {
    fn default() -> Self {
        MockDeck::new()
    }
}

impl MockDeck {
    pub fn new() -> MockDeck {
        let hello = Hello {
            product: "touchdeck".into(),
            proto: td_core::PROTOCOL,
            elf_sha256: MOCK_ELF_SHA256.into(),
            version: "mock".into(),
            running: "ota_0".into(),
            sinks: vec!["log".into(), "link".into()],
            recovery: false,
            layout: "builtin".into(),
        };
        MockDeck {
            state: Arc::new(Mutex::new(State {
                hello,
                v2: false,
                selected_v2: false,
                device_name: "Open TouchDeck".into(),
                gif_capabilities: false,
                media_capabilities: false,
                ble: MockBle::default(),
                radio: false,
                settings: Settings::default(),
                icons: BTreeSet::new(),
                icon_files: HashMap::new(),
                widgets: BTreeMap::new(),
                dashboards: BTreeMap::new(),
                sets: Vec::new(),
                received: Vec::new(),
                lines: Vec::new(),
                silent: false,
                reply_delay: Duration::ZERO,
                lose_console_replies: 0,
                frames_rx: 0,
                frames_tx: 0,
                dropped: 0,
                last_rx: None,
                online_since_hello: false,
                refuse_sessions: false,
                booted: Instant::now(),
                link: None,
                link_network: false,
                generation: 0,
                layouts: HashMap::new(),
                shown: Orientation::Landscape,
                last_xfer: 0,
                last_seq: 0,
                inbound: None,
                outbound: None,
                lose_in: Vec::new(),
                lose_out: Vec::new(),
                corrupt_next_put: false,
                reboots: 0,
                key_seq: 0,
                press_seq: 0,
                input_seq: 0,
                update_sha: None,
                update_pending: None,
                restart: None,
                restart_window: None,
                update_mode: String::new(),
                update_aborted: false,
                assets: BTreeMap::new(),
                asset_capacity: ASSET_CAPACITY,
                pinned_assets: BTreeSet::new(),
                chunk_bytes: 0,
                asset_end: None,
                live: Live::new(),
                live_sets: Vec::new(),
                live_time: None,
                asset_refusals: VecDeque::new(),
                idle_settings: IdleSettings::default(),
                idle: false,
                photo_interval: None,
                last_photo: None,
                photos: Vec::new(),
                photo_refusals: Vec::new(),
                photo_end: None,
                clock: None,
                page: 0,
                page_reports: true,
                selections: 0,
                page_sent: None,
                wifi: MockWifi::default(),
            })),
        }
    }

    pub fn set_gif_capabilities(&self, enabled: bool) {
        self.state().gif_capabilities = enabled;
    }

    /// Advertises the volume and media caps a v2 hello will carry: `bar` and `now-playing` Tiles, text live values
    /// up to 128 bytes, four streams a Tile, JPEG cover Assets up to 256 pixels a side, and four cover Tiles a
    /// Layout. Off by default, like Firmware without them.
    pub fn set_media_capabilities(&self, enabled: bool) {
        self.state().media_capabilities = enabled;
    }

    /// Runs the radio build, or the normal build again. A v2 hello says which.
    pub fn set_radio(&self, radio: bool) {
        self.state().radio = radio;
    }

    pub fn radio(&self) -> bool {
        self.state().radio
    }

    pub fn v2() -> Self {
        let device = Self::new();
        device.state().v2 = true;
        device
    }

    /// Makes this a radio build: it answers `wifi.*` and a v2 hello advertises `caps.wifi`. Off by default, like the
    /// normal build, which answers `unsupported`.
    pub fn set_wifi_capability(&self, enabled: bool) {
        self.state().wifi.capability = enabled;
    }

    /// Puts a made-up network in range. A set with this SSID and password joins; another password fails with
    /// `wrong-password`, and an SSID not in range with `not-found`.
    pub fn add_wifi_network(&self, ssid: &str, password: &str) {
        self.state()
            .wifi
            .networks
            .insert(ssid.into(), password.into());
    }

    /// Answers every `wifi.*` request with `usb-only`, as the Device does on a network Carrier.
    pub fn set_wifi_usb_only(&self, usb_only: bool) {
        self.state().wifi.usb_only = usb_only;
    }

    /// The stored network and its password, as the Device's NVS holds them.
    pub fn wifi_network(&self) -> Option<(String, String)> {
        self.state().wifi.stored.clone()
    }

    /// Sets the name a v2 hello reports, as a Device someone already named.
    pub fn set_name(&self, name: &str) {
        self.state().device_name = name.into();
    }

    /// Answers `hello` as a board running other Firmware with this product.
    pub fn set_product(&self, product: &str) {
        self.state().hello.product = product.into();
    }

    pub fn usb_serial(&self) -> &'static str {
        "02:00:00:00:00:01"
    }

    /// Makes this a radio build with a Bluetooth keyboard that keeps `bonds` bonds: a v2 hello advertises
    /// `caps.ble_hid` and the Device answers `ble.*`. `None`, the default, is the normal build, which doesn't know
    /// those messages. Bluetooth starts off, as on a fresh Device.
    pub fn set_ble_capability(&self, bonds: Option<u32>) {
        let mut state = self.state();
        state.ble.bonds_max = bonds;
        state.hello.sinks.retain(|s| s != "hid");
        if bonds.is_some() {
            state.hello.sinks.push("hid".into());
        }
    }

    /// Puts a bond in the Device's bond store, as a host that paired earlier.
    pub fn add_ble_bond(&self, address: &str, random: bool, connected: bool) {
        let mut state = self.state();
        state.ble.bonds.retain(|bond| bond.address != address);
        state.ble.bonds.push(MockBond {
            address: address.into(),
            random,
            connected,
        });
    }

    /// The bond store, whether Bluetooth is on or off: `(address, random, connected)` each.
    pub fn ble_bonds(&self) -> Vec<(String, bool, bool)> {
        self.state()
            .ble
            .bonds
            .iter()
            .map(|bond| (bond.address.clone(), bond.random, bond.connected))
            .collect()
    }

    /// Whether Bluetooth's stored setting is on.
    pub fn ble_enabled(&self) -> bool {
        self.state().ble.enabled
    }

    /// Makes the next starts fail, as a controller that won't come up: `ble.set` answers `radio`.
    pub fn fail_ble_start(&self, fail: bool) {
        self.state().ble.start_fails = fail;
    }

    /// Holds the host unsynced, so Bluetooth keeps answering `starting`; `false` lets it sync before the next request.
    /// Without a hold the host syncs between requests, so `ble.set true` answers `starting` and the next request `on`.
    pub fn ble_hold_start(&self, hold: bool) {
        self.state().ble.hold_start = hold;
    }

    /// The Bluetooth host resets and starts again: `starting`, with the bonds still listed.
    pub fn ble_host_reset(&self) {
        self.state().ble.host_reset();
    }

    /// Makes the Device's storage writes fail: changing the setting or forgetting a bond answers `storage`.
    pub fn fail_ble_storage(&self, fail: bool) {
        self.state().ble.storage_fails = fail;
    }

    /// A host starts pairing inside the window, so the Device shows a passkey; `false` takes it down.
    pub fn ble_show_passkey(&self, shown: bool) {
        let mut state = self.state();
        state.ble.passkey = shown && state.ble.window_until.is_some();
    }

    /// A host types the passkey and bonds inside the open window, which closes it. `false` when no window is open.
    pub fn ble_host_pairs(&self, address: &str, random: bool) -> bool {
        self.state().ble.host_pairs(address, random)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Simulates the ROM flasher writing a validated bundle, preserving Layout storage. The board
    /// then runs Firmware that speaks protocol 2.
    pub fn flash_bundle(&self, bundle: &crate::bundle::Bundle) {
        let mut state = self.state();
        state.silent = false;
        state.v2 = true;
        state.hello.product = "touchdeck".into();
        state.hello.version = bundle.manifest.version.clone();
        state.hello.elf_sha256 = bundle.manifest.elf_sha256.clone();
        state.hello.running = "ota_0".into();
        state.radio = bundle.manifest.radio;
        state.update_aborted = false;
        state.reboots += 1;
    }
    pub fn update_mode(&self, mode: &str) {
        self.state().update_mode = mode.into();
    }
    /// Makes the next Firmware update restart like a real Device: after `update.end` the deck keeps
    /// answering as the old build for `drop_after`, then its port is gone for `down_for`, then it
    /// comes back on the new build.
    pub fn restart_after(&self, drop_after: Duration, down_for: Duration) {
        self.state().restart = Some((drop_after, down_for));
    }
    /// When the port of the restarting deck goes away, once an update has been sent.
    pub fn port_drops_at(&self) -> Option<Instant> {
        self.state().restart_window.map(|(drop_at, _)| drop_at)
    }
    /// Plugs a host into the deck. A later connect replaces the earlier one. While the port is
    /// gone the transport reads end-of-file at once.
    pub fn connect(&self) -> MockTransport {
        self.try_connect().unwrap_or_else(|_| {
            let (to_deck, _) = mpsc::channel::<Vec<u8>>();
            let (_, from_deck) = mpsc::channel::<Vec<u8>>();
            MockTransport { from_deck, to_deck }
        })
    }
    /// Like [`MockDeck::connect`], but fails the way opening a vanished port does.
    pub fn try_connect(&self) -> io::Result<MockTransport> {
        {
            let mut state = self.state();
            let now = Instant::now();
            let window = state
                .restart_window
                .filter(|_| state.update_pending.is_some());
            let still_up = window.is_some_and(|(drop_at, _)| now < drop_at);
            if let Some((drop_at, back_at)) = window
                && (drop_at..back_at).contains(&now)
            {
                return Err(io::Error::from(ErrorKind::NotFound));
            }
            if !still_up && let Some((hash, slot, radio)) = state.update_pending.take() {
                state.reboots += 1;
                if state.update_mode == "rollback" {
                    state.update_aborted = true;
                } else {
                    state.hello.elf_sha256 = hash;
                    state.hello.running = slot;
                    state.radio = radio;
                }
                state.booted = Instant::now();
            }
        }
        Ok(self.attach(false))
    }

    /// Links a network carrier, as the fake net-v2 Device does once a session is ready. It
    /// replaces whatever link there was, and a later USB connect replaces it.
    #[cfg(feature = "fake")]
    pub(crate) fn connect_network(&self) -> MockTransport {
        self.attach(true)
    }

    fn attach(&self, network: bool) -> MockTransport {
        let (to_deck, from_host) = mpsc::channel::<Vec<u8>>();
        let (to_host, from_deck) = mpsc::channel::<Vec<u8>>();
        let generation = {
            let mut state = self.state();
            state.selected_v2 = false;
            state.generation += 1;
            state.link = Some(to_host.clone());
            state.link_network = network;
            state.generation
        };
        let deck = self.clone();
        thread::spawn(move || deck.run(&from_host, &to_host, generation));
        MockTransport { from_deck, to_deck }
    }

    /// Whether the USB link is online: a USB host said `hello` and sent a frame within 5 s.
    /// net-v2 refuses a network session then.
    pub fn usb_online(&self) -> bool {
        let state = self.state();
        state.link.is_some()
            && !state.link_network
            && state.online_since_hello
            && state.last_rx.is_some_and(|at| at.elapsed() < ONLINE_FOR)
    }

    /// The id a v2 `hello` reports: the factory MAC that is also the USB serial.
    pub fn device_id(&self) -> [u8; 6] {
        [0x02, 0, 0, 0, 0, 0x01]
    }

    /// Stops answering anything for good, as a deck that hung or lost its cable without an error.
    pub fn stop(&self) {
        self.state().silent = true;
    }

    /// Ends the USB stream, as a port that went away. A network link stays up.
    pub fn hang_up(&self) {
        let mut state = self.state();
        if state.link_network {
            return;
        }
        state.link = None;
        state.generation += 1;
    }

    /// Drops the session as the Firmware's link idle timeout does: the deck keeps answering, but
    /// reports `online: false` and refuses protocol 2 `not-negotiated` until the next `hello`.
    pub fn drop_session(&self) {
        let mut state = self.state();
        state.selected_v2 = false;
        state.online_since_hello = false;
    }

    /// While on, `hello` still answers but no longer brings the link online, so the deck refuses
    /// every session it is offered.
    pub fn refuse_sessions(&self, refuse: bool) {
        self.state().refuse_sessions = refuse;
    }

    pub fn delay_replies(&self, delay: Duration) {
        self.state().reply_delay = delay;
    }

    /// Reports uptime as if the deck booted `uptime` ago, like a Device that ran before the
    /// host connected.
    pub fn set_uptime(&self, uptime: Duration) {
        self.state().booted = Instant::now()
            .checked_sub(uptime)
            .expect("the host clock reaches back that far");
    }

    /// Drops the next `count` console replies, as input or output lost on the wire.
    pub fn lose_console_replies(&self, count: u32) {
        self.state().lose_console_replies = count;
    }

    /// Shows the Page at `index` of the Layout on screen, as a swipe or a page action does.
    pub fn show_page(&self, index: usize) {
        self.state().page = index;
    }

    /// While off, the deck never sends `v2.page`, like Firmware that predates it. On by default.
    pub fn set_page_reports(&self, enabled: bool) {
        self.state().page_reports = enabled;
    }

    pub fn emit(&self, event: &Event) {
        self.emit_value(&serde_json::to_value(event).expect("events serialize"));
    }

    /// Sends any event object, including ones `Event` does not name, such as `key.resync`.
    pub fn emit_value(&self, event: &Value) {
        let state = self.state();
        if let (false, Some(link)) = (state.silent, &state.link) {
            let payload = serde_json::to_vec(event).expect("events serialize");
            let _ = link.send(
                Frame::message(Role::Event, payload)
                    .encode()
                    .expect("events fit one frame"),
            );
        }
    }

    fn uptime_ms(&self) -> u64 {
        self.state().booted.elapsed().as_millis() as u64
    }

    /// Emits a key report the way the link sink does, with the next `seq`. `report` holds
    /// `type` and either `mods` and `keys` or `usage`.
    pub fn emit_key(&self, report: &Value) -> u32 {
        let seq = self.next_key_seq();
        let mut event = Map::from_iter([
            ("m".to_owned(), json!("key.report")),
            ("seq".to_owned(), json!(seq)),
            ("t".to_owned(), json!(self.uptime_ms())),
        ]);
        for field in ["type", "mods", "keys", "usage"] {
            if let Some(value) = report.get(field) {
                event.insert(field.to_owned(), value.clone());
            }
        }
        self.emit_value(&Value::Object(event));
        seq
    }

    /// Emits `key.resync` with the engine's state, as the link sink does after its queue overflowed.
    pub fn emit_resync(&self, mods: u8, keys: &[u8]) -> u32 {
        let seq = self.next_key_seq();
        let t = self.uptime_ms();
        self.emit_value(
            &json!({"m": "key.resync", "seq": seq, "t": t, "mods": mods, "keys": keys}),
        );
        seq
    }

    /// Spends `count` key `seq` numbers without sending, as reports the link sink dropped.
    pub fn drop_key_reports(&self, count: u32) {
        self.state().key_seq += count;
    }

    pub fn emit_press(&self, page: &str, tile: &str, kind: &str) -> u32 {
        let seq = {
            let mut state = self.state();
            state.press_seq += 1;
            state.press_seq
        };
        self.emit(&Event::Press {
            page: page.into(),
            tile: tile.into(),
            kind: kind.into(),
            part: None,
            seq,
            t: self.uptime_ms(),
        });
        seq
    }

    /// Emits a tap that lifted inside a renderer control. Only a v2 session hears the `part`, as on the Firmware.
    pub fn emit_press_part(&self, page: &str, tile: &str, kind: &str, part: &str) -> u32 {
        let (seq, v2) = {
            let mut state = self.state();
            state.press_seq += 1;
            (state.press_seq, state.selected_v2)
        };
        self.emit(&Event::Press {
            page: page.into(),
            tile: tile.into(),
            kind: kind.into(),
            part: v2.then(|| part.into()),
            seq,
            t: self.uptime_ms(),
        });
        seq
    }

    /// Emits one drag snapshot, its value rounded to 4 decimals as the Firmware sends it, and returns its `seq`.
    /// A v1 session gets nothing, so this returns `None`.
    pub fn emit_input(
        &self,
        page: &str,
        tile: &str,
        stream: &str,
        phase: InputPhase,
        value: f64,
    ) -> Option<u32> {
        let seq = {
            let mut state = self.state();
            if !state.selected_v2 {
                return None;
            }
            state.input_seq += 1;
            state.input_seq
        };
        self.emit(&Event::Input {
            page: page.into(),
            tile: tile.into(),
            stream: stream.into(),
            phase,
            value: (value * 10_000.0).round() / 10_000.0,
            seq,
            t: self.uptime_ms(),
        });
        Some(seq)
    }

    fn next_key_seq(&self) -> u32 {
        let mut state = self.state();
        state.key_seq += 1;
        state.key_seq
    }

    pub fn add_icons(&self, ids: impl IntoIterator<Item = String>) {
        self.state().icons.extend(ids);
    }

    /// The icon ids the deck stores.
    pub fn icons(&self) -> BTreeSet<String> {
        self.state().icons.clone()
    }

    /// The file an `icon.put` stored under `id`.
    pub fn icon_file(&self, id: &str) -> Option<Vec<u8>> {
        self.state().icon_files.get(id).cloned()
    }

    pub fn received(&self) -> Vec<Value> {
        self.state().received.clone()
    }

    pub fn lines(&self) -> Vec<String> {
        self.state().lines.clone()
    }

    pub fn settings(&self) -> Settings {
        self.state().settings
    }

    /// The latest `widget.set` per tile.
    pub fn widgets(&self) -> BTreeMap<String, Value> {
        self.state().widgets.clone()
    }

    /// The latest `dashboard.set` per page.
    pub fn dashboards(&self) -> BTreeMap<String, Value> {
        self.state().dashboards.clone()
    }

    /// Every `widget.set` and `dashboard.set` in arrival order, each with `at_ms`, the unix
    /// milliseconds at which the mock received it.
    pub fn sets(&self) -> Vec<Value> {
        self.state().sets.clone()
    }

    /// Queues `count` refusals with `reason`. The oldest queued refusal answers the next Asset request when that
    /// request uses its `method`.
    pub fn refuse_assets(&self, method: &str, reason: &str, count: usize) {
        self.state()
            .asset_refusals
            .extend((0..count).map(|_| (method.into(), reason.into())));
    }

    /// Sets the elapsed time used by live-value rate windows.
    pub fn set_live_time(&self, milliseconds: u64) {
        self.state().live_time = Some(milliseconds);
    }

    /// Every `v2.live.set` the deck accepted, in order, each with the `at_ms` it arrived.
    pub fn live_sets(&self) -> Vec<Value> {
        self.state().live_sets.clone()
    }

    /// The live stream `(page, tile, stream)` of the Layout on screen, once a value for it was accepted.
    pub fn live(&self, page: &str, tile: &str, stream: &str) -> Option<MockLive> {
        let state = self.state();
        let online =
            state.online_since_hello && state.last_rx.is_some_and(|at| at.elapsed() < ONLINE_FOR);
        let key = (page.to_owned(), tile.to_owned(), stream.to_owned());
        state
            .live
            .slots
            .get(&key)
            .filter(|slot| slot.n > 0)
            .map(|slot| MockLive {
                n: slot.n,
                samples: slot.samples.clone(),
                value: slot.value.clone(),
                stale: !online || slot.expires.is_some_and(|at| at <= Instant::now()),
            })
    }

    /// The `layout` a live request must name for the Layout on screen, and that Layout.
    pub fn live_layout(&self) -> (String, Layout) {
        let state = self.state();
        match state
            .layouts
            .get(&state.shown)
            .and_then(|bytes| Layout::parse(bytes).ok().map(|layout| (bytes, layout)))
        {
            Some((bytes, layout)) => (layout_ref(bytes), layout),
            None => (
                crate::live::BUILTIN.into(),
                Layout::parse(BUILTIN).expect("builtin.json is valid"),
            ),
        }
    }

    /// The layout file the deck stores for `orientation`, as it arrived.
    pub fn layout(&self, orientation: Orientation) -> Option<Vec<u8>> {
        self.state().layouts.get(&orientation).cloned()
    }

    pub fn reboots(&self) -> u32 {
        self.state().reboots
    }

    /// Seeds a stored Asset and optionally makes deletion return `in-use`.
    pub fn seed_asset(&self, bytes: Vec<u8>, in_use: bool) -> String {
        let id = asset_id(&bytes);
        let mut state = self.state();
        state.assets.insert(id.clone(), bytes);
        if in_use {
            state.pinned_assets.insert(id.clone());
        }
        id
    }

    /// The Assets the Device stores, by id.
    pub fn assets(&self) -> BTreeMap<String, Vec<u8>> {
        self.state().assets.clone()
    }

    /// Sets the bytes the Device has for Assets, so a test can fill it.
    pub fn set_asset_capacity(&self, bytes: u64) {
        self.state().asset_capacity = bytes;
    }

    /// Puts the Device on its idle screen, as `timeout_s` without a touch does, or wakes it, as a
    /// touch does. A wake cancels the picture on its way, and its end then answers `idle`.
    pub fn set_idle(&self, idle: bool) {
        let mut state = self.state();
        if state.idle
            && !idle
            && let Some(Inbound {
                target: Target::Photo(_),
                xfer,
                ..
            }) = state.inbound
        {
            state.inbound = None;
            state.photo_end = Some((xfer, refusal("idle", "The Device woke")));
        }
        if idle && !state.idle {
            state.last_photo = None;
        }
        state.idle = idle;
    }

    /// Paces pictures at `every` instead of the stored `interval_s`, so a test need not wait 5 s.
    pub fn set_photo_interval(&self, every: Duration) {
        self.state().photo_interval = Some(every);
    }

    /// Refuses the next puts with these reasons, in order, before any other check. `retry_ms`
    /// goes with `rate`.
    pub fn refuse_photos(&self, refusals: impl IntoIterator<Item = (&'static str, Option<u64>)>) {
        self.state().photo_refusals.extend(
            refusals
                .into_iter()
                .map(|(reason, ms)| (reason.to_owned(), ms)),
        );
    }

    /// Every picture the Device showed, in order.
    pub fn photos(&self) -> Vec<Vec<u8>> {
        self.state().photos.clone()
    }

    pub fn idle_settings(&self) -> IdleSettings {
        self.state().idle_settings
    }

    /// The last `v2.clock.set`, as `(unix_s, offset_min)`.
    pub fn clock(&self) -> Option<(u64, i32)> {
        self.state().clock
    }

    /// The data bytes of every chunk frame the Device has received.
    pub fn chunk_bytes(&self) -> usize {
        self.state().chunk_bytes
    }

    /// Drops arriving chunks with these indices, one listed index per arrival.
    pub fn lose_chunks(&self, indices: impl IntoIterator<Item = u16>) {
        self.state().lose_in.extend(indices);
    }

    /// Drops the deck's own chunk sends with these indices, one listed index per send.
    pub fn lose_sent_chunks(&self, indices: impl IntoIterator<Item = u16>) {
        self.state().lose_out.extend(indices);
    }

    /// Flips one bit of the next put's data, so its CRC-32 fails at `xfer.end`.
    pub fn corrupt_next_put(&self) {
        self.state().corrupt_next_put = true;
    }

    /// The `layout state` fields for the layout the deck shows in its boot orientation.
    pub fn layout_state(&self) -> LayoutState {
        let state = self.state();
        let stored = state.layouts.get(&state.shown).and_then(|bytes| {
            Layout::parse(bytes)
                .ok()
                .map(|layout| (layout, bytes.len()))
        });
        let (source, (layout, bytes)) = match stored {
            Some(stored) => ("storage", stored),
            None => (
                "builtin",
                (
                    Layout::parse(BUILTIN).expect("builtin.json is valid"),
                    BUILTIN.len(),
                ),
            ),
        };
        let mut tiles = 0;
        let mut icons = BTreeSet::new();
        for page in &layout.pages {
            if let Page::Grid { tiles: list, .. } = page {
                tiles += list.len() as u32;
                icons.extend(list.iter().filter_map(|tile| {
                    tile.icon
                        .as_deref()
                        .filter(|icon| icon.starts_with("file:"))
                }));
            }
        }
        let missing = icons
            .iter()
            .filter(|icon| !state.icons.contains(&icon[5..]))
            .count() as u32;
        LayoutState {
            source: source.into(),
            name: layout.name.clone(),
            orientation: layout.orientation,
            pages: layout.pages.len() as u32,
            tiles,
            icons_loaded: Some(icons.len() as u32 - missing),
            icons_missing: Some(missing),
            bytes: Some(bytes as u32),
            load_ms: Some(0),
            error: Some(String::new()),
        }
    }

    fn next_xfer(state: &mut State) -> u8 {
        state.last_xfer = state.last_xfer % 255 + 1;
        state.last_xfer
    }

    /// Sends the chunks of a get that are due: new ones inside the window and ones whose
    /// ack is 200 ms late. A chunk sent 5 times without an ack aborts the get.
    fn report_page(&self, to_host: &Sender<Vec<u8>>) {
        let wire = {
            let mut state = self.state();
            if !(state.page_reports && state.v2 && state.selected_v2 && state.online_since_hello) {
                return;
            }
            let reference = state.layouts.get(&state.shown).map_or_else(
                || crate::live::BUILTIN.to_owned(),
                |bytes| layout_ref(bytes),
            );
            let now = (state.selections, reference, state.page);
            if state.page_sent.as_ref() == Some(&now) {
                return;
            }
            let layout = state
                .layouts
                .get(&state.shown)
                .and_then(|bytes| Layout::parse(bytes).ok())
                .unwrap_or_else(|| Layout::parse(BUILTIN).expect("builtin.json is valid"));
            let Some(page) = layout.pages.get(state.page) else {
                return;
            };
            let event = Event::Page {
                page: page.id().to_owned(),
            };
            state.page_sent = Some(now);
            let payload = serde_json::to_vec(&event).expect("events serialize");
            Frame::message(Role::Event, payload)
                .encode()
                .expect("events fit one frame")
        };
        let _ = to_host.send(wire);
    }

    fn pump(&self, to_host: &Sender<Vec<u8>>) {
        let mut wires = Vec::new();
        {
            let mut guard = self.state();
            let state = &mut *guard;
            let Some(out) = state.outbound.as_mut() else {
                return;
            };
            let now = Instant::now();
            let mut due: Vec<u16> = (out.base..out.ahead)
                .filter(|idx| {
                    out.sent
                        .get(idx)
                        .is_some_and(|(at, _)| *at + RETRY_AFTER <= now)
                })
                .collect();
            while out.ahead < out.count() && out.ahead < out.base + WINDOW {
                due.push(out.ahead);
                out.ahead += 1;
            }
            for idx in due {
                let times = out.sent.get(&idx).map_or(0, |(_, times)| *times);
                if times >= TRANSMISSIONS {
                    state.outbound = None;
                    return;
                }
                out.sent.insert(idx, (now, times + 1));
                if let Some(at) = state.lose_out.iter().position(|lost| *lost == idx) {
                    state.lose_out.remove(at);
                    continue;
                }
                state.last_seq = state.last_seq % 255 + 1;
                let frame = Frame {
                    role: Role::Request(state.last_seq),
                    body: Body::Chunk {
                        xfer: out.xfer,
                        idx,
                        data: out.chunk(idx),
                    },
                };
                wires.push(frame.encode().expect("chunks fit one frame"));
                state.frames_tx += 1;
            }
        }
        for wire in wires {
            let _ = to_host.send(wire);
        }
    }

    /// Takes one chunk of a put, or one ack of a get, and returns the wire bytes to answer.
    fn answer_xfer(&self, role: Role, body: Body) -> Vec<u8> {
        let mut guard = self.state();
        let state = &mut *guard;
        state.frames_rx += 1;
        state.last_rx = Some(Instant::now());
        match (role, body) {
            (Role::Request(seq), Body::Chunk { xfer, idx, data }) => {
                state.chunk_bytes += data.len();
                let Some(inbound) = state
                    .inbound
                    .as_mut()
                    .filter(|inbound| inbound.xfer == xfer)
                else {
                    state.dropped += 1;
                    return Vec::new();
                };
                if let Some(at) = state.lose_in.iter().position(|lost| *lost == idx) {
                    state.lose_in.remove(at);
                    return Vec::new();
                }
                if idx == inbound.next && inbound.data.len() + data.len() <= inbound.bytes {
                    inbound.data.extend_from_slice(&data);
                    inbound.next += 1;
                }
                let ack = Frame {
                    role: Role::Response(seq),
                    body: Body::Ack {
                        xfer,
                        next: inbound.next,
                    },
                };
                state.frames_tx += 1;
                ack.encode().expect("acks fit one frame")
            }
            (Role::Response(_), Body::Ack { xfer, next }) => {
                if let Some(out) = state.outbound.as_mut().filter(|out| out.xfer == xfer)
                    && next > out.base
                {
                    out.base = next.min(out.ahead);
                    let base = out.base;
                    out.sent.retain(|idx, _| *idx >= base);
                }
                Vec::new()
            }
            _ => {
                state.dropped += 1;
                Vec::new()
            }
        }
    }

    fn layout_put(&self, request: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let orientation = orientation_of(request)?;
        let bytes = request
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or("layout.put needs bytes")? as usize;
        let crc32 = request
            .get("crc32")
            .and_then(Value::as_u64)
            .and_then(|crc| u32::try_from(crc).ok())
            .ok_or("layout.put needs crc32")?;
        if bytes > UPLOAD_MAX {
            return Err(format!("layout is {bytes} bytes; maximum is {UPLOAD_MAX}"));
        }
        let mut state = self.state();
        let xfer = MockDeck::next_xfer(&mut state);
        state.inbound = Some(Inbound {
            xfer,
            target: Target::Layout(orientation),
            bytes,
            crc32,
            data: Vec::with_capacity(bytes),
            next: 0,
        });
        Ok(Map::from_iter([("xfer".to_owned(), json!(xfer))]))
    }

    fn icon_put(&self, request: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let id = icon_id_of(request)?;
        let bytes = request
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or("icon.put needs bytes")? as usize;
        let crc32 = request
            .get("crc32")
            .and_then(Value::as_u64)
            .and_then(|crc| u32::try_from(crc).ok())
            .ok_or("icon.put needs crc32")?;
        if bytes != ICON_BYTES {
            return Err(format!("an icon is {ICON_BYTES} bytes, not {bytes}"));
        }
        let mut state = self.state();
        let xfer = MockDeck::next_xfer(&mut state);
        state.inbound = Some(Inbound {
            xfer,
            target: Target::Icon(id),
            bytes,
            crc32,
            data: Vec::with_capacity(bytes),
            next: 0,
        });
        Ok(Map::from_iter([("xfer".to_owned(), json!(xfer))]))
    }

    fn icon_rm(&self, request: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let id = icon_id_of(request)?;
        let mut state = self.state();
        if !state.icons.remove(&id) {
            return Err(format!("no icon {id}"));
        }
        state.icon_files.remove(&id);
        Ok(Map::new())
    }

    fn layout_get(&self, request: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let orientation = orientation_of(request)?;
        let mut state = self.state();
        let data = state.layouts.get(&orientation).cloned().ok_or_else(|| {
            format!(
                "no stored layout for {}",
                request["orientation"].as_str().unwrap_or_default()
            )
        })?;
        let xfer = MockDeck::next_xfer(&mut state);
        let fields = json!({"xfer": xfer, "bytes": data.len(), "crc32": crc32(&data)});
        state.outbound = Some(Outbound {
            xfer,
            data,
            base: 0,
            ahead: 0,
            sent: HashMap::new(),
        });
        Ok(fields.as_object().cloned().unwrap_or_default())
    }

    fn xfer_end(&self, request: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let xfer = request
            .get("xfer")
            .and_then(Value::as_u64)
            .ok_or("xfer.end needs xfer")?;
        let mut state = self.state();
        if state
            .outbound
            .as_ref()
            .is_some_and(|out| u64::from(out.xfer) == xfer)
        {
            state.outbound = None;
            return Ok(Map::new());
        }
        if let Some(Inbound {
            target: Target::Asset(..) | Target::Photo(_),
            xfer: open,
            ..
        }) = &state.inbound
        {
            return Err(if u64::from(*open) == xfer {
                "end an Asset or a picture with its v2 end".into()
            } else {
                "no transfer with that xfer".into()
            });
        }
        let inbound = state
            .inbound
            .take()
            .filter(|inbound| u64::from(inbound.xfer) == xfer)
            .ok_or("no transfer with that xfer")?;
        let mut data = inbound.data;
        if std::mem::take(&mut state.corrupt_next_put)
            && let Some(byte) = data.first_mut()
        {
            *byte ^= 1;
        }
        if data.len() != inbound.bytes {
            return Err(format!(
                "received {} of {} bytes",
                data.len(),
                inbound.bytes
            ));
        }
        if crc32(&data) != inbound.crc32 {
            return Err("crc32 mismatch".into());
        }
        let orientation = match inbound.target {
            Target::Layout(orientation) => orientation,
            Target::Update => return Err("firmware uses update.end".into()),
            Target::Asset(..) | Target::Photo(_) => unreachable!("a v2 put returned above"),
            Target::Icon(id) => {
                if !data.starts_with(b"TDI1") {
                    return Err("not a TDI1 icon".into());
                }
                state.icons.insert(id.clone());
                state.icon_files.insert(id.clone(), data);
                return Ok(Map::from_iter([
                    ("id".to_owned(), json!(id)),
                    ("bytes".to_owned(), json!(ICON_BYTES)),
                ]));
            }
        };
        let layout =
            Layout::parse(&data).map_err(|errors| format!("invalid layout: {}", errors[0]))?;
        let covers = layout
            .pages
            .iter()
            .filter_map(|page| match page {
                Page::Grid { tiles, .. } => Some(tiles),
                Page::Dashboard { .. } => None,
            })
            .flatten()
            .filter(|tile| {
                tile.streams
                    .as_ref()
                    .is_some_and(|streams| streams.values().any(|kind| *kind == StreamType::Image))
            })
            .count();
        let limit = if state.media_capabilities {
            MEDIA_LIVE_IMAGES
        } else {
            0
        };
        if covers > limit {
            return Err(format!(
                "size: {covers} Tiles declare an image stream; this Device draws {limit}"
            ));
        }
        if state.gif_capabilities && over_page_budget(&state, &layout) {
            return Ok(refusal("size", "Page over its animation budget"));
        }
        state.layouts.insert(orientation, data);
        if orientation == state.shown {
            state.hello.layout = "storage".into();
            state.live.slots.clear();
            state.page = 0;
        }
        drop(state);
        let fields = serde_json::to_value(self.layout_state()).expect("layout state serializes");
        Ok(fields.as_object().cloned().unwrap_or_default())
    }

    fn run(&self, from_host: &Receiver<Vec<u8>>, to_host: &Sender<Vec<u8>>, generation: u64) {
        let mut decoder = StreamDecoder::default();
        let mut line = Vec::new();
        let mut heartbeat = (Instant::now(), 0u64);
        loop {
            if self.state().generation != generation {
                return;
            }
            let bytes = match from_host.recv_timeout(READ_TICK) {
                Ok(bytes) => bytes,
                Err(RecvTimeoutError::Timeout) => Vec::new(),
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if self.state().silent {
                continue;
            }
            if heartbeat.0.elapsed() >= HEARTBEAT {
                let uptime = self.state().booted.elapsed().as_millis();
                let _ = to_host.send(
                    format!(
                        "I ({uptime}) touchdeck: alive {}, free heap 4194304 bytes\r\n",
                        heartbeat.1
                    )
                    .into_bytes(),
                );
                heartbeat = (Instant::now(), heartbeat.1 + 1);
            }
            for item in decoder.feed(&bytes, Instant::now()) {
                let out = match item {
                    Item::Frame(Frame {
                        role,
                        body: body @ (Body::Chunk { .. } | Body::Ack { .. }),
                    }) => self.answer_xfer(role, body),
                    Item::Frame(frame) => self.answer_frame(frame),
                    Item::Text(text) => self.answer_text(&text, &mut line),
                };
                if !out.is_empty() {
                    let delay = self.state().reply_delay;
                    thread::sleep(delay);
                    // A cable pulled while the Device was answering takes the reply with it.
                    if self.state().generation != generation || to_host.send(out).is_err() {
                        return;
                    }
                }
            }
            self.pump(to_host);
            self.report_page(to_host);
        }
    }

    fn answer_text(&self, text: &[u8], line: &mut Vec<u8>) -> Vec<u8> {
        let mut out = Vec::new();
        for &byte in text {
            if byte != b'\n' && byte != b'\r' {
                line.push(byte);
                continue;
            }
            let command = String::from_utf8_lossy(line).into_owned();
            line.clear();
            let Some(word) = command.split(' ').find(|word| !word.is_empty()) else {
                continue;
            };
            let reply = match word {
                "layout" if command.split_whitespace().eq(["layout", "state"]) => {
                    let mut reply =
                        serde_json::to_value(self.layout_state()).expect("layout state serializes");
                    reply["cmd"] = json!("layout");
                    reply["op"] = json!("state");
                    reply["ok"] = json!(true);
                    reply
                }
                "update" => {
                    let state = self.state();
                    json!({"cmd":"update","op":"state","ok":true,"running":state.hello.running,
                        "confirmed":state.update_mode!="no-confirm","ota_0":if state.update_aborted && state.hello.running=="ota_1" {"ABORTED"} else {"VALID"},"ota_1":if state.update_aborted && state.hello.running=="ota_0" {"ABORTED"} else {"VALID"}})
                }
                "version" => {
                    let hello = self.state().hello.clone();
                    json!({"cmd": "version", "ok": true, "project_name": "touchdeck", "version": hello.version, "elf_sha256": hello.elf_sha256})
                }
                "reboot" => {
                    let mut state = self.state();
                    state.reboots += 1;
                    state.shown = state.settings.orientation;
                    state.page = 0;
                    state.booted = Instant::now();
                    state.online_since_hello = false;
                    state.inbound = None;
                    state.outbound = None;
                    state.hello.layout = if state.layouts.contains_key(&state.shown) {
                        "storage".into()
                    } else {
                        "builtin".into()
                    };
                    json!({"cmd": "reboot", "ok": true})
                }
                "link" => {
                    let mut reply = Map::from_iter([
                        ("cmd".to_owned(), json!("link")),
                        ("ok".to_owned(), json!(true)),
                    ]);
                    reply.extend(self.link_state());
                    Value::Object(reply)
                }
                _ => json!({"cmd": word, "ok": false, "error": "unknown command"}),
            };
            {
                let mut state = self.state();
                state.lines.push(command.clone());
                if state.lose_console_replies > 0 {
                    state.lose_console_replies -= 1;
                    continue;
                }
            }
            out.extend(format!("@td {reply}\r\ntd> ").into_bytes());
        }
        out
    }

    fn link_state(&self) -> Map<String, Value> {
        let state = self.state();
        let online =
            state.online_since_hello && state.last_rx.is_some_and(|at| at.elapsed() < ONLINE_FOR);
        let last_rx_ms = state
            .last_rx
            .map_or(0, |at| at.duration_since(state.booted).as_millis() as u64);
        let fields = json!({
            "online": online, "frames_rx": state.frames_rx, "frames_tx": state.frames_tx,
            "crc_errors": 0, "dropped": state.dropped, "last_rx_ms": last_rx_ms,
        });
        fields.as_object().cloned().unwrap_or_default()
    }

    fn answer_frame(&self, frame: Frame) -> Vec<u8> {
        {
            let mut state = self.state();
            state.frames_rx += 1;
            if state.last_rx.is_some_and(|at| at.elapsed() >= ONLINE_FOR) {
                state.selected_v2 = false;
                state.online_since_hello = false;
            }
            state.last_rx = Some(Instant::now());
        }
        let (Role::Request(seq), Body::Message(payload)) = (frame.role, frame.body) else {
            self.state().dropped += 1;
            return Vec::new();
        };
        let request = match serde_json::from_slice::<Value>(&payload) {
            Ok(Value::Object(request)) if request.get("m").is_some_and(Value::is_string) => request,
            _ => {
                self.state().dropped += 1;
                return Vec::new();
            }
        };
        let mut recorded = request.clone();
        if let Some(password) = recorded.get_mut("password") {
            // The Device never keeps a request's password anywhere but NVS, and neither does this record.
            *password = json!("(redacted)");
        }
        self.state().received.push(Value::Object(recorded));
        let m = request["m"].as_str().unwrap_or_default().to_owned();
        let fields = self.dispatch(&m, &request);
        let mut reply = Map::from_iter([
            ("m".to_owned(), json!(m)),
            ("ok".to_owned(), json!(fields.is_ok())),
        ]);
        match fields {
            Ok(fields) => reply.extend(fields),
            Err(error) => {
                reply.insert("error".into(), json!(error));
            }
        }
        let payload = serde_json::to_vec(&reply).expect("replies serialize");
        let wire = Frame::message(Role::Response(seq), payload)
            .encode()
            .expect("mock replies fit one frame");
        self.state().frames_tx += 1;
        wire
    }

    fn dispatch(
        &self,
        m: &str,
        request: &Map<String, Value>,
    ) -> Result<Map<String, Value>, String> {
        let object = |value: Value| value.as_object().cloned().unwrap_or_default();
        match m {
            "hello" => {
                let mut state = self.state();
                state.online_since_hello = !state.refuse_sessions;
                let mut reply =
                    object(serde_json::to_value(&state.hello).expect("hello serializes"));
                if state.v2 {
                    if request.keys().any(|k| k != "m" && k != "proto")
                        || request.get("proto").is_some_and(|p| p.as_u64().is_none())
                    {
                        return Ok(object(
                            json!({"ok":false,"reason":"args","error":"Invalid protocol selector"}),
                        ));
                    }
                    let proto = request.get("proto").and_then(Value::as_u64).unwrap_or(1);
                    if proto != 1 && proto != 2 {
                        return Ok(object(
                            json!({"ok":false,"reason":"unsupported","error":"Unsupported protocol"}),
                        ));
                    }
                    state.selected_v2 = proto == 2;
                    state.selections += u64::from(proto == 2);
                    state.live = Live::new();
                    if matches!(
                        &state.inbound,
                        Some(Inbound {
                            target: Target::Asset(..) | Target::Photo(_),
                            ..
                        })
                    ) {
                        state.inbound = None;
                    }
                    reply.insert("protocols".into(), json!([1, 2]));
                    if proto == 2 {
                        reply.insert("proto".into(), json!(2));
                        reply.insert("id".into(), json!("020000000001"));
                        reply.insert("name".into(), json!(state.device_name));
                        reply.insert("caps".into(), state.capabilities());
                        if state.radio {
                            reply.insert("radio".into(), json!(true));
                        }
                    }
                }
                Ok(reply)
            }
            m if m.starts_with("wifi.") => self.wifi(m, request),
            m if m.starts_with("v2.asset.") && self.v2_selected() => Ok(self.asset(m, request)),
            "v2.live.set" if self.v2_selected() => Ok(self.live_set(request)),
            "v2.photo.put" | "v2.photo.end" | "v2.idle.get" | "v2.idle.set" | "v2.clock.set"
                if self.v2_selected() =>
            {
                Ok(self.photo(m, request))
            }
            m if m.starts_with("v2.") && self.state().v2 => {
                let mut state = self.state();
                let reason = if !state.selected_v2 || !state.online_since_hello {
                    Some("not-negotiated")
                } else if m != "v2.name.get" && m != "v2.name.set" {
                    Some("unsupported")
                } else if request
                    .keys()
                    .any(|k| k != "m" && !(m == "v2.name.set" && k == "name"))
                    || (m == "v2.name.set"
                        && !request
                            .get("name")
                            .and_then(Value::as_str)
                            .is_some_and(crate::identity::valid_name))
                {
                    Some("args")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    return Ok(object(
                        json!({"ok":false,"reason":reason,"error":"Request refused"}),
                    ));
                }
                if m == "v2.name.set" {
                    state.device_name = request["name"].as_str().unwrap().into();
                }
                Ok(object(json!({"name":state.device_name})))
            }
            "ping" => {
                let t = self.state().booted.elapsed().as_millis() as u64;
                Ok(object(
                    json!({"t": t, "n": request.get("n").cloned().unwrap_or(Value::Null)}),
                ))
            }
            "link.state" => Ok(self.link_state()),
            m if m.starts_with("ble.") => {
                let mut state = self.state();
                let network = state.link_network;
                state.ble.handle(m, request, network)
            }
            "icon.list" => {
                let state = self.state();
                let cursor = request.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;
                let ids: Vec<&String> = state.icons.iter().skip(cursor).take(ICON_PAGE).collect();
                let next = (cursor + ids.len() < state.icons.len()).then_some(cursor + ids.len());
                Ok(object(json!({"ids": ids, "next": next})))
            }
            "settings.get" => Ok(object(
                serde_json::to_value(self.state().settings).expect("settings serialize"),
            )),
            "settings.put" => {
                let mut state = self.state();
                let settings = Settings::from_value(&Value::Object(request.clone()));
                let reboot_required = settings.orientation != state.settings.orientation;
                state.settings = settings;
                let mut fields =
                    object(serde_json::to_value(settings).expect("settings serialize"));
                fields.insert("reboot_required".into(), json!(reboot_required));
                Ok(fields)
            }
            "widget.set" => {
                let tile = request
                    .get("tile")
                    .and_then(Value::as_str)
                    .ok_or("widget.set needs a tile".to_owned())?;
                let mut state = self.state();
                state
                    .widgets
                    .insert(tile.to_owned(), Value::Object(request.clone()));
                state.record_set(request);
                Ok(Map::new())
            }
            "dashboard.set" => {
                let page = request
                    .get("page")
                    .and_then(Value::as_str)
                    .ok_or("dashboard.set needs a page")?;
                let rows = request
                    .get("rows")
                    .and_then(Value::as_array)
                    .ok_or("dashboard.set needs rows")?;
                if rows.len() > DASHBOARD_ROWS {
                    return Err("a dashboard holds 8 rows".into());
                }
                let mut state = self.state();
                state
                    .dashboards
                    .insert(page.to_owned(), Value::Object(request.clone()));
                state.record_set(request);
                Ok(Map::new())
            }
            "update.begin" => {
                let bytes = request
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .filter(|b| *b > 0 && *b <= 0x400000)
                    .ok_or("invalid update bytes")? as usize;
                let hash = request
                    .get("sha256")
                    .and_then(Value::as_str)
                    .ok_or("missing update sha256")?
                    .to_owned();
                let mut state = self.state();
                if state.inbound.is_some() {
                    return Err("transfer busy".into());
                }
                state.last_xfer = state.last_xfer % 255 + 1;
                let xfer = state.last_xfer;
                let slot = if state.hello.running == "ota_0" {
                    "ota_1"
                } else {
                    "ota_0"
                };
                state.inbound = Some(Inbound {
                    xfer,
                    target: Target::Update,
                    bytes,
                    crc32: 0,
                    data: Vec::new(),
                    next: 0,
                });
                state.update_sha = Some(hash);
                state.update_aborted = false;
                Ok(object(json!({"xfer":xfer,"slot":slot})))
            }
            "update.end" => {
                let mut state = self.state();
                let inbound = state.inbound.take().ok_or("no update")?;
                if inbound.target != Target::Update
                    || request.get("xfer").and_then(Value::as_u64) != Some(u64::from(inbound.xfer))
                    || inbound.data.len() != inbound.bytes
                    || Some(crate::bundle::sha256(&inbound.data)) != state.update_sha.take()
                {
                    return Err("update sha256 or size mismatch".into());
                }
                if inbound.data.len() < 208 {
                    return Err("invalid application image".into());
                }
                let hash = inbound.data[176..208]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                let slot = if state.hello.running == "ota_0" {
                    "ota_1"
                } else {
                    "ota_0"
                };
                // Real Firmware knows what it was built as. The mock can't, so it takes an image whose app descriptor
                // version ends in `-radio` as a radio build, as tests/fixtures/touchdeck-test-radio.tdfw is made.
                let radio = inbound.data[48..80]
                    .split(|b| *b == 0)
                    .next()
                    .is_some_and(|version| version.ends_with(b"-radio"));
                state.update_pending = Some((hash, slot.into(), radio));
                if let Some((drop_after, down_for)) = state.restart {
                    let drop_at = Instant::now() + drop_after;
                    state.restart_window = Some((drop_at, drop_at + down_for));
                }
                Ok(object(json!({"slot":slot})))
            }
            "layout.put" => self.layout_put(request),
            "icon.put" => self.icon_put(request),
            "icon.rm" => self.icon_rm(request),
            "layout.get" => self.layout_get(request),
            "xfer.end" => self.xfer_end(request),
            _ => Err("unknown message".into()),
        }
    }
}

fn jpeg_size(data: &[u8]) -> Option<(u16, u16)> {
    if !data.starts_with(&[0xFF, 0xD8]) || !data.ends_with(&[0xFF, 0xD9]) {
        return None;
    }
    let mut at = 2;
    let mut size = None;
    loop {
        if *data.get(at)? != 0xFF {
            return None;
        }
        let marker = *data.get(at + 1)?;
        if marker == 0xFF {
            at += 1;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([*data.get(at + 2)?, *data.get(at + 3)?]));
        let segment = data.get(at + 4..at + 2 + length).filter(|_| length >= 2)?;
        match marker {
            0xC0 => {
                let [precision, h0, h1, w0, w1, components, ..] = *segment else {
                    return None;
                };
                let (w, h) = (u16::from_be_bytes([w0, w1]), u16::from_be_bytes([h0, h1]));
                if precision != 8 || components != 3 || w == 0 || h == 0 {
                    return None;
                }
                size = Some((w, h));
            }
            0xC1..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => return None,
            0xDA => return size,
            _ => {}
        }
        at += 2 + length;
    }
}

fn over_page_budget(state: &State, layout: &Layout) -> bool {
    layout.pages.iter().any(|page| {
        let Page::Grid { tiles, .. } = page else {
            return false;
        };
        let animated: Vec<_> = tiles
            .iter()
            .filter(|tile| tile.render == Some(TileRender::Gif))
            .collect();
        let pixels: u64 = animated
            .iter()
            .filter_map(|tile| tile.asset.as_deref()?.strip_prefix("sha256:"))
            .filter_map(|id| state.assets.get(id))
            .filter_map(|gif| td_host::gif_normalize::inspect_normalized(gif).ok())
            .map(|info| u64::from(info.width) * u64::from(info.height))
            .sum();
        animated.len() as u64 > MOCK_PAGE_TILES || pixels > MOCK_PAGE_PIXELS
    })
}

fn asset_space(state: &State) -> (u64, u64, u64, u64) {
    let used: u64 = state.assets.values().map(|data| data.len() as u64).sum();
    let reserved = match &state.inbound {
        Some(Inbound {
            target: Target::Asset(..),
            bytes,
            ..
        }) => *bytes as u64,
        _ => 0,
    };
    let total = state.asset_capacity;
    (total, used, reserved, total.saturating_sub(used + reserved))
}

impl MockDeck {
    fn wifi(&self, m: &str, request: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let only = |keys: &[&str]| {
            request
                .keys()
                .all(|key| key == "m" || keys.contains(&key.as_str()))
        };
        let mut state = self.state();
        // The Device checks the Carrier before anything else: off USB, every `wifi.*` is refused.
        let off_usb = state.link_network;
        let wifi = &mut state.wifi;
        if wifi.usb_only || off_usb {
            return Ok(refusal("usb-only", "Only over USB"));
        }
        // Without a station the Device has no `wifi.*` messages at all, as with `ble.*` without `ble_hid`.
        if !wifi.capability {
            return Err("unknown message".into());
        }
        match m {
            "wifi.state" if only(&[]) => wifi.resolve(),
            "wifi.forget" if only(&[]) => {
                *wifi = MockWifi {
                    capability: wifi.capability,
                    networks: std::mem::take(&mut wifi.networks),
                    ..MockWifi::default()
                }
            }
            "wifi.set" if only(&["ssid", "password"]) => {
                let ssid = request.get("ssid").and_then(Value::as_str);
                let password = request.get("password").and_then(Value::as_str);
                let (Some(ssid), Some(password)) = (ssid, password) else {
                    return Ok(refusal("args", "Invalid network name or password"));
                };
                if !valid_ssid(ssid) || !valid_password(&WifiPassword::new(password)) {
                    return Ok(refusal("args", "Invalid network name or password"));
                }
                wifi.stored = Some((ssid.into(), password.into()));
                wifi.joining = true;
                wifi.outcome = None;
            }
            "wifi.state" | "wifi.forget" | "wifi.set" => {
                return Ok(refusal("args", "Invalid request fields"));
            }
            _ => return Ok(refusal("unsupported", "Unsupported request")),
        }
        Ok(wifi.fields())
    }

    fn v2_selected(&self) -> bool {
        let state = self.state();
        state.v2 && state.selected_v2 && state.online_since_hello
    }

    fn asset(&self, m: &str, request: &Map<String, Value>) -> Map<String, Value> {
        let refused = |reason: &str| refusal(reason, "Request refused");
        {
            let mut state = self.state();
            if state
                .asset_refusals
                .front()
                .is_some_and(|(method, _)| method == m)
            {
                return refused(&state.asset_refusals.pop_front().unwrap().1);
            }
        }
        let only = |keys: &[&str]| {
            request
                .keys()
                .all(|key| key == "m" || keys.contains(&key.as_str()))
        };
        let id_field = || {
            request
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| valid_asset_id(id))
                .map(str::to_owned)
        };
        let mut guard = self.state();
        let state = &mut *guard;
        match m {
            "v2.asset.stat" => {
                if !only(&[]) {
                    return refused("args");
                }
                let (total, used, reserved, free) = asset_space(state);
                Map::from_iter([
                    ("total_bytes".to_owned(), json!(total)),
                    ("used_bytes".to_owned(), json!(used)),
                    ("reserved_bytes".to_owned(), json!(reserved)),
                    ("free_bytes".to_owned(), json!(free)),
                ])
            }
            "v2.asset.list" => {
                let after = match request.get("after") {
                    Some(Value::Null) => None,
                    Some(Value::String(id)) if valid_asset_id(id) => Some(id.clone()),
                    _ => return refused("args"),
                };
                if !only(&["after"]) {
                    return refused("args");
                }
                let mut rest = state
                    .assets
                    .iter()
                    .filter(|(id, _)| after.as_ref().is_none_or(|after| *id > after));
                let items: Vec<Value> = rest
                    .by_ref()
                    .take(ASSET_PAGE)
                    .map(|(id, data)| json!({"id": id, "bytes": data.len(), "format": AssetFormat::sniff(data).map(AssetFormat::name)}))
                    .collect();
                let next = match rest.next() {
                    Some(_) => items.last().map(|item| item["id"].clone()),
                    None => None,
                };
                Map::from_iter([
                    ("items".to_owned(), json!(items)),
                    ("next".to_owned(), next.unwrap_or(Value::Null)),
                ])
            }
            "v2.asset.put" => {
                let (Some(id), Some(bytes), Some(format)) = (
                    id_field(),
                    request
                        .get("bytes")
                        .and_then(Value::as_u64)
                        .filter(|bytes| *bytes > 0),
                    request.get("format").and_then(Value::as_str),
                ) else {
                    return refused("args");
                };
                if !only(&["id", "bytes", "format"]) {
                    return refused("args");
                }
                let format = match AssetFormat::parse(format) {
                    Some(AssetFormat::Tdi1) => AssetFormat::Tdi1,
                    Some(AssetFormat::Gif) if state.gif_capabilities => AssetFormat::Gif,
                    Some(AssetFormat::Jpeg) if state.media_capabilities => AssetFormat::Jpeg,
                    _ => return refused("format"),
                };
                if bytes > state.asset_bytes() as u64 {
                    return refused("size");
                }
                if let Some(stored) = state.assets.get(&id) {
                    if stored.len() as u64 != bytes || AssetFormat::sniff(stored) != Some(format) {
                        return refused("format");
                    }
                    return Map::from_iter([
                        ("present".to_owned(), json!(true)),
                        ("xfer".to_owned(), Value::Null),
                    ]);
                }
                if state.inbound.is_some() {
                    return refused("busy");
                }
                if bytes > asset_space(state).3 {
                    return refused("full");
                }
                let xfer = MockDeck::next_xfer(state);
                state.asset_end = None;
                state.inbound = Some(Inbound {
                    xfer,
                    target: Target::Asset(id, format),
                    bytes: bytes as usize,
                    crc32: 0,
                    data: Vec::with_capacity(bytes as usize),
                    next: 0,
                });
                Map::from_iter([
                    ("present".to_owned(), json!(false)),
                    ("xfer".to_owned(), json!(xfer)),
                ])
            }
            "v2.asset.end" => {
                let Some(xfer) = request
                    .get("xfer")
                    .and_then(Value::as_u64)
                    .filter(|xfer| (1..=255).contains(xfer))
                    .filter(|_| only(&["xfer"]))
                else {
                    return refused("args");
                };
                let open = matches!(&state.inbound,
                    Some(Inbound { target: Target::Asset(..), xfer: open, .. }) if u64::from(*open) == xfer);
                if !open {
                    return match &state.asset_end {
                        Some((done, reply)) if u64::from(*done) == xfer => reply.clone(),
                        _ => refused("state"),
                    };
                }
                let inbound = state.inbound.take().expect("the Asset put is open");
                if inbound.data.len() < inbound.bytes {
                    state.inbound = Some(inbound);
                    return refused("incomplete");
                }
                let Target::Asset(id, format) = inbound.target else {
                    unreachable!("matched an Asset put")
                };
                let checked = match AssetFormat::sniff(&inbound.data) {
                    found if found != Some(format) => Err("format"),
                    Some(AssetFormat::Tdi1) if inbound.data.len() == ICON_BYTES => Ok(()),
                    Some(AssetFormat::Jpeg) => match jpeg_size(&inbound.data) {
                        Some((w, h)) if w > MEDIA_IMAGE || h > MEDIA_IMAGE => Err("size"),
                        Some(_) => Ok(()),
                        None => Err("format"),
                    },
                    Some(AssetFormat::Gif) if state.gif_capabilities => {
                        td_host::gif_normalize::inspect_normalized(&inbound.data)
                            .ok()
                            .filter(|info| {
                                info.width <= 128
                                    && info.height <= 128
                                    && info.frames <= 60
                                    && info.min_delay_ms >= 100
                                    && usize::from(info.width)
                                        * usize::from(info.height)
                                        * info.frames
                                        <= 983040
                            })
                            .map(|_| ())
                            .ok_or("format")
                    }
                    _ => Err("format"),
                };
                let reply = if asset_id(&inbound.data) != id {
                    refused("hash")
                } else if let Err(reason) = checked {
                    refused(reason)
                } else {
                    let bytes = inbound.data.len();
                    state.assets.insert(id.clone(), inbound.data);
                    Map::from_iter([
                        ("id".to_owned(), json!(id)),
                        ("bytes".to_owned(), json!(bytes)),
                    ])
                };
                state.asset_end = Some((inbound.xfer, reply.clone()));
                reply
            }
            "v2.asset.delete" => {
                let Some(id) = id_field().filter(|_| only(&["id"])) else {
                    return refused("args");
                };
                if matches!(&state.inbound,
                    Some(Inbound { target: Target::Asset(open, _), .. }) if *open == id)
                {
                    return refused("busy");
                }
                if state.shows(&id) || state.pinned_assets.contains(&id) {
                    return refused("in-use");
                }
                let deleted = state.assets.remove(&id).is_some();
                Map::from_iter([("deleted".to_owned(), json!(deleted))])
            }
            _ => refused("unsupported"),
        }
    }
}

impl MockDeck {
    fn live_set(&self, request: &Map<String, Value>) -> Map<String, Value> {
        const FIELDS: [&str; 9] = [
            "m", "layout", "page", "tile", "stream", "n", "ttl_ms", "type", "value",
        ];
        let refused = |reason: &str| refusal(reason, "Request refused");
        let text = |key: &str| request.get(key).and_then(Value::as_str);
        let whole = |key: &str| {
            request
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
        };
        let kind = request
            .get("type")
            .and_then(|kind| serde_json::from_value::<StreamType>(kind.clone()).ok());
        let value = request.get("value").cloned().unwrap_or(Value::Null);
        let numbers: Option<Vec<f64>> = match kind {
            Some(StreamType::Number) => value.as_f64().map(|v| vec![v]),
            Some(StreamType::Series) => value
                .as_array()
                .and_then(|items| items.iter().map(Value::as_f64).collect()),
            Some(StreamType::Text) => value.is_string().then(Vec::new),
            Some(StreamType::Image) => (value.is_null() || value.is_string()).then(Vec::new),
            None => None,
        };
        let (
            Some(layout),
            Some(page),
            Some(tile),
            Some(stream),
            Some(n),
            Some(ttl),
            Some(kind),
            Some(numbers),
        ) = (
            text("layout"),
            text("page"),
            text("tile"),
            text("stream"),
            whole("n"),
            whole("ttl_ms"),
            kind,
            numbers,
        )
        else {
            return refusal("args", "Invalid live value");
        };
        if request.keys().any(|k| !FIELDS.contains(&k.as_str())) || !request.contains_key("value") {
            return refusal("args", "Invalid live value");
        }
        if n == 0 || !(TTL_MIN_MS..=TTL_MAX_MS).contains(&ttl) {
            return refused("args");
        }
        let (reference, active) = self.live_layout();
        if layout != reference {
            return refused("layout");
        }
        let Some(found) = active.pages.iter().find(|p| p.id() == page) else {
            return refused("page");
        };
        let Some(found) = (match found {
            Page::Grid { tiles, .. } => tiles.iter().find(|t| t.id == tile),
            _ => None,
        }) else {
            return refused("tile");
        };
        let Some(declared) = found.streams.as_ref().and_then(|s| s.get(stream)) else {
            return refused("stream");
        };
        let media = self.state().media_capabilities;
        if *declared != kind || !(matches!(kind, StreamType::Number | StreamType::Series) || media)
        {
            return refused("type");
        }
        match (kind, &value) {
            (StreamType::Text, Value::String(text)) => {
                if text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}') {
                    return refused("args");
                }
                if text.len() > MEDIA_TEXT_BYTES {
                    return refused("size");
                }
            }
            (StreamType::Image, Value::String(reference)) => {
                let Some(id) = reference
                    .strip_prefix("sha256:")
                    .filter(|id| valid_asset_id(id))
                else {
                    return refused("args");
                };
                let state = self.state();
                let cover = state.assets.get(id).is_some_and(|data| {
                    AssetFormat::sniff(data) == Some(AssetFormat::Jpeg)
                        && jpeg_size(data)
                            .is_some_and(|(w, h)| w <= MEDIA_IMAGE && h <= MEDIA_IMAGE)
                });
                if !cover {
                    return refused("asset");
                }
            }
            (StreamType::Image, _) => {}
            _ if numbers.is_empty() => return refused("args"),
            _ => {}
        }
        if numbers.len() > MOCK_SERIES {
            return refused("size");
        }
        if numbers
            .iter()
            .any(|v| !v.is_finite() || v.abs() > VALUE_MAX)
        {
            return refused("args");
        }
        if found.render == Some(td_core::layout::TileRender::Bar)
            && stream == "value"
            && numbers.iter().any(|v| !(0.0..=1.0).contains(v))
        {
            return refused("args");
        }
        let sent = json!([ttl, kind, value]);
        let mut state = self.state();
        let elapsed = state
            .live_time
            .unwrap_or_else(|| state.live.session.elapsed().as_millis() as u64);
        let window = elapsed / 1000;
        let all = if state.live.window == window {
            state.live.in_window
        } else {
            0
        };
        let key = (page.to_owned(), tile.to_owned(), stream.to_owned());
        let slot = state.live.slots.entry(key).or_default();
        if n < slot.n || (n == slot.n && sent != slot.sent) {
            return refused("sequence");
        }
        if n == slot.n {
            return Map::from_iter([("n".to_owned(), json!(n))]);
        }
        let mine = if slot.window == window {
            slot.in_window
        } else {
            0
        };
        if mine >= MOCK_LIVE_HZ || all >= MOCK_LIVE_TOTAL_HZ {
            let mut reply = refused("rate");
            reply.insert("retry_ms".into(), json!(1000 - elapsed % 1000));
            return reply;
        }
        slot.window = window;
        slot.in_window = mine + 1;
        slot.n = n;
        slot.sent = sent;
        slot.value = value;
        slot.expires = Some(Instant::now() + Duration::from_millis(u64::from(ttl)));
        if kind == StreamType::Series {
            slot.samples = numbers;
        } else {
            slot.samples.extend(numbers);
            let over = slot.samples.len().saturating_sub(MOCK_SERIES);
            slot.samples.drain(..over);
        }
        state.live.window = window;
        state.live.in_window = all + 1;
        state.record_live(request);
        Map::from_iter([("n".to_owned(), json!(n))])
    }

    fn photo(&self, m: &str, request: &Map<String, Value>) -> Map<String, Value> {
        let refused = |reason: &str| refusal(reason, "Request refused");
        let only = |keys: &[&str]| {
            request.len() == keys.len() + 1
                && request
                    .keys()
                    .all(|key| key == "m" || keys.contains(&key.as_str()))
        };
        let idle_fields = |settings: IdleSettings| {
            Map::from_iter([
                ("enabled".to_owned(), json!(settings.enabled)),
                ("timeout_s".to_owned(), json!(settings.timeout_s)),
                ("interval_s".to_owned(), json!(settings.interval_s)),
                ("fallback".to_owned(), json!("clock")),
            ])
        };
        let mut guard = self.state();
        let state = &mut *guard;
        match m {
            "v2.idle.get" => {
                if !only(&[]) {
                    return refused("args");
                }
                idle_fields(state.idle_settings)
            }
            "v2.idle.set" => {
                let number = |key: &str| {
                    request
                        .get(key)
                        .and_then(Value::as_u64)
                        .and_then(|n| u32::try_from(n).ok())
                };
                let (Some(enabled), Some(timeout_s), Some(interval_s), Some("clock")) = (
                    request.get("enabled").and_then(Value::as_bool),
                    number("timeout_s").filter(|n| IDLE_TIMEOUT_S.contains(n)),
                    number("interval_s").filter(|n| IDLE_INTERVAL_S.contains(n)),
                    request.get("fallback").and_then(Value::as_str),
                ) else {
                    return refused("args");
                };
                if !only(&["enabled", "timeout_s", "interval_s", "fallback"]) {
                    return refused("args");
                }
                state.idle_settings = IdleSettings {
                    enabled,
                    timeout_s,
                    interval_s,
                };
                if !enabled {
                    state.idle = false;
                }
                idle_fields(state.idle_settings)
            }
            "v2.clock.set" => {
                let (Some(unix_s), Some(offset_min)) = (
                    request
                        .get("unix_s")
                        .and_then(Value::as_u64)
                        .filter(|n| *n <= crate::photo::CLOCK_UNIX_S_MAX),
                    request
                        .get("offset_min")
                        .and_then(Value::as_i64)
                        .and_then(|n| i32::try_from(n).ok())
                        .filter(|n| crate::photo::CLOCK_OFFSET_MIN.contains(n)),
                ) else {
                    return refused("args");
                };
                if !only(&["unix_s", "offset_min"]) {
                    return refused("args");
                }
                state.clock = Some((unix_s, offset_min));
                Map::from_iter([
                    ("unix_s".to_owned(), json!(unix_s)),
                    ("offset_min".to_owned(), json!(offset_min)),
                ])
            }
            "v2.photo.put" => {
                let (Some(bytes), Some(sha256)) = (
                    request
                        .get("bytes")
                        .and_then(Value::as_u64)
                        .filter(|bytes| *bytes > 0),
                    request
                        .get("sha256")
                        .and_then(Value::as_str)
                        .filter(|id| valid_asset_id(id)),
                ) else {
                    return refused("args");
                };
                if !only(&["bytes", "sha256"]) {
                    return refused("args");
                }
                if !state.photo_refusals.is_empty() {
                    let (reason, retry_ms) = state.photo_refusals.remove(0);
                    let mut reply = refused(&reason);
                    if let Some(ms) = retry_ms {
                        reply.insert("retry_ms".into(), json!(ms));
                    }
                    return reply;
                }
                if bytes > PHOTO_BYTES_MAX as u64 {
                    return refused("size");
                }
                if !state.idle_settings.enabled || !state.idle {
                    return refusal("idle", "The Device is awake");
                }
                let every = state
                    .photo_interval
                    .unwrap_or(Duration::from_secs(u64::from(
                        state.idle_settings.interval_s,
                    )));
                if let Some(wait) = state
                    .last_photo
                    .map(|at| (at + every).saturating_duration_since(Instant::now()))
                    .filter(|wait| !wait.is_zero())
                {
                    let mut reply = refusal("rate", "Too soon for the next picture");
                    reply.insert(
                        "retry_ms".into(),
                        json!(wait.as_micros().div_ceil(1000) as u64),
                    );
                    return reply;
                }
                if state.inbound.is_some() {
                    return refused("busy");
                }
                let xfer = MockDeck::next_xfer(state);
                state.photo_end = None;
                state.inbound = Some(Inbound {
                    xfer,
                    target: Target::Photo(sha256.to_owned()),
                    bytes: bytes as usize,
                    crc32: 0,
                    data: Vec::with_capacity(bytes as usize),
                    next: 0,
                });
                Map::from_iter([("xfer".to_owned(), json!(xfer))])
            }
            "v2.photo.end" => {
                let Some(xfer) = request
                    .get("xfer")
                    .and_then(Value::as_u64)
                    .filter(|xfer| (1..=255).contains(xfer))
                    .filter(|_| only(&["xfer"]))
                else {
                    return refused("args");
                };
                let open = matches!(&state.inbound,
                    Some(Inbound { target: Target::Photo(_), xfer: open, .. }) if u64::from(*open) == xfer);
                if !open {
                    return match &state.photo_end {
                        Some((done, reply)) if u64::from(*done) == xfer => reply.clone(),
                        _ => refused("state"),
                    };
                }
                let inbound = state.inbound.take().expect("the photo put is open");
                if inbound.data.len() < inbound.bytes {
                    state.inbound = Some(inbound);
                    return refused("incomplete");
                }
                let Target::Photo(sha256) = &inbound.target else {
                    unreachable!("matched a photo put")
                };
                let reply = if asset_id(&inbound.data) != *sha256 {
                    refused("hash")
                } else if AssetFormat::sniff(&inbound.data) != Some(AssetFormat::Jpeg) {
                    refused("format")
                } else if !state.idle {
                    refusal("idle", "The Device woke")
                } else {
                    state.photos.push(inbound.data);
                    state.last_photo = Some(Instant::now());
                    Map::from_iter([("shown".to_owned(), json!(true))])
                };
                state.photo_end = Some((inbound.xfer, reply.clone()));
                reply
            }
            _ => refused("unsupported"),
        }
    }
}

fn orientation_of(request: &Map<String, Value>) -> Result<Orientation, String> {
    match request.get("orientation").and_then(Value::as_str) {
        Some("landscape") => Ok(Orientation::Landscape),
        Some("portrait") => Ok(Orientation::Portrait),
        _ => Err("orientation must be landscape or portrait".into()),
    }
}

fn icon_id_of(request: &Map<String, Value>) -> Result<String, String> {
    request
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| {
            id.len() == 16
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .map(str::to_owned)
        .ok_or_else(|| "id must be 16 lowercase hex digits".into())
}

pub struct MockTransport {
    from_deck: Receiver<Vec<u8>>,
    to_deck: Sender<Vec<u8>>,
}

impl Transport for MockTransport {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        Ok((
            Box::new(MockReader {
                from_deck: self.from_deck,
                held: Vec::new(),
            }),
            Box::new(MockWriter {
                to_deck: self.to_deck,
            }),
        ))
    }
}

struct MockReader {
    from_deck: Receiver<Vec<u8>>,
    held: Vec<u8>,
}

impl Read for MockReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.held.is_empty() {
            match self.from_deck.recv_timeout(READ_TICK) {
                Ok(bytes) => self.held = bytes,
                Err(RecvTimeoutError::Timeout) => return Err(ErrorKind::TimedOut.into()),
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let n = buf.len().min(self.held.len());
        buf[..n].copy_from_slice(&self.held[..n]);
        self.held.drain(..n);
        Ok(n)
    }
}

struct MockWriter {
    to_deck: Sender<Vec<u8>>,
}

impl Write for MockWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.to_deck
            .send(buf.to_vec())
            .map_err(|_| io::Error::from(ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Session, SessionConfig};
    use td_core::Orientation;

    #[test]
    fn layout_state_reports_declared_orientation_for_mismatched_storage_and_builtin() {
        let deck = MockDeck::new();
        deck.state().shown = Orientation::Portrait;
        assert_eq!(deck.layout_state().source, "builtin");
        assert_eq!(deck.layout_state().orientation, Orientation::Landscape);
        // A legacy or external writer may have placed landscape JSON in the portrait slot.
        deck.state()
            .layouts
            .insert(Orientation::Portrait, BUILTIN.to_vec());
        let shown = deck.layout_state();
        assert_eq!(shown.source, "storage");
        assert_eq!(shown.orientation, Orientation::Landscape);
    }

    #[test]
    fn settings_put_stores_firmware_values_and_flags_an_orientation_change() {
        let deck = MockDeck::new();
        let (session, _events) = Session::open(
            Box::new(deck.connect()),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .unwrap();
        let put = td_core::Request::SettingsPut(Settings {
            idle_timeout_s: 120,
            orientation: Orientation::Portrait,
        });
        let reply = session.request(&put, Duration::from_secs(1)).unwrap();
        assert_eq!(reply.fields.get("reboot_required"), Some(&json!(true)));
        assert_eq!(
            deck.settings(),
            Settings {
                idle_timeout_s: 120,
                orientation: Orientation::Portrait
            }
        );
        assert_eq!(deck.received().last().unwrap()["m"], "settings.put");
    }

    #[test]
    fn link_state_counts_frames_and_hello_brings_the_link_online() {
        let deck = MockDeck::new();
        let (session, _events) = Session::open(
            Box::new(deck.connect()),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .unwrap();
        let state: td_core::LinkState = session
            .request(&td_core::Request::LinkState, Duration::from_secs(1))
            .unwrap()
            .fields()
            .unwrap();
        assert_eq!(
            (state.frames_rx, state.frames_tx, state.online),
            (1, 0, false)
        );
        session.hello(Duration::from_secs(1)).unwrap();
        let state: td_core::LinkState = session
            .request(&td_core::Request::LinkState, Duration::from_secs(1))
            .unwrap()
            .fields()
            .unwrap();
        assert_eq!(
            (state.frames_rx, state.frames_tx, state.online),
            (3, 2, true)
        );
    }

    #[test]
    fn key_reports_count_seq_and_a_drop_leaves_a_gap() {
        let deck = MockDeck::new();
        let (_session, events) = Session::open(
            Box::new(deck.connect()),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .unwrap();
        deck.emit_key(&json!({"type": "kbd", "mods": 0, "keys": [104]}));
        deck.drop_key_reports(1);
        deck.emit_key(&json!({"type": "con", "usage": 233}));
        deck.emit_resync(0, &[]);
        deck.emit_press("apps", "launch", "launch");
        let got: Vec<Value> = (0..4)
            .map(
                |_| match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                    crate::Incoming::Event(event) => event,
                    other => panic!("not an event: {other:?}"),
                },
            )
            .collect();
        assert_eq!(
            (
                got[0]["m"].as_str(),
                got[0]["seq"].as_u64(),
                &got[0]["keys"]
            ),
            (Some("key.report"), Some(1), &json!([104]))
        );
        assert_eq!(
            (got[1]["seq"].as_u64(), &got[1]["usage"]),
            (Some(3), &json!(233))
        );
        assert_eq!(
            (got[2]["m"].as_str(), got[2]["seq"].as_u64()),
            (Some("key.resync"), Some(4))
        );
        assert_eq!(
            (
                got[3]["m"].as_str(),
                got[3]["seq"].as_u64(),
                got[3]["tile"].as_str()
            ),
            (Some("press"), Some(1), Some("launch"))
        );
    }

    #[test]
    fn widget_and_dashboard_sets_are_recorded_in_order_with_arrival_times() {
        let deck = MockDeck::new();
        let (session, _events) = Session::open(
            Box::new(deck.connect()),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .unwrap();
        let widget = td_core::Request::WidgetSet {
            tile: "context".into(),
            value: "42%".into(),
            caption: "Context".into(),
            bar: 0.42,
            stale: false,
        };
        let dashboard = td_core::Request::DashboardSet {
            page: "accounts".into(),
            rows: vec![td_core::DashboardRow {
                label: "a@b.c".into(),
                value: "active".into(),
                bar: -1.0,
            }],
        };
        assert!(session.request(&widget, Duration::from_secs(1)).unwrap().ok);
        assert!(
            session
                .request(&dashboard, Duration::from_secs(1))
                .unwrap()
                .ok
        );
        let sets = deck.sets();
        assert_eq!(
            sets.iter().map(|set| set["m"].clone()).collect::<Vec<_>>(),
            [json!("widget.set"), json!("dashboard.set")]
        );
        assert_eq!(sets[0]["tile"], "context");
        assert_eq!(sets[0]["value"], "42%");
        assert!(sets[0]["at_ms"].as_u64().unwrap() <= sets[1]["at_ms"].as_u64().unwrap());
        assert_eq!(deck.widgets()["context"]["stale"], false);
        assert_eq!(deck.dashboards()["accounts"]["rows"][0]["label"], "a@b.c");
    }

    #[test]
    fn a_ninth_dashboard_row_is_refused() {
        let deck = MockDeck::new();
        let rows: Vec<Value> = (0..9)
            .map(|n| json!({"label": n.to_string(), "value": "x", "bar": -1}))
            .collect();
        let request = json!({"m": "dashboard.set", "page": "p", "rows": rows});
        assert_eq!(
            deck.dispatch("dashboard.set", request.as_object().unwrap()),
            Err("a dashboard holds 8 rows".to_owned())
        );
        assert!(deck.sets().is_empty());
    }

    #[test]
    fn an_unknown_message_is_refused() {
        let deck = MockDeck::new();
        assert_eq!(
            deck.dispatch("layout.nope", &Map::new()),
            Err("unknown message".to_owned())
        );
    }

    #[test]
    fn a_named_v2_device_and_other_firmware_answer_as_set() {
        let deck = MockDeck::v2();
        deck.set_name("Desk left");
        let (session, _) =
            Session::open(Box::new(deck.connect()), SessionConfig::default()).unwrap();
        let identity = session.identity(Duration::from_secs(1)).unwrap();
        assert_eq!(identity.name.as_deref(), Some("Desk left"));
        let other = MockDeck::new();
        other.set_product("esp-at");
        assert!(matches!(
            crate::probe_transport(Box::new(other.connect())),
            crate::Probe::NotDeck { detail } if detail.contains("esp-at")
        ));
    }
}

#[cfg(test)]
mod jpeg_contract_tests {
    #[test]
    fn grayscale_is_not_a_device_jpeg() {
        let gray = [
            0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0, 0xff, 0xda, 0, 2, 0xff,
            0xd9,
        ];
        assert_eq!(super::jpeg_size(&gray), None);
    }
}
