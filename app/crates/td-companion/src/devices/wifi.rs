//! Wi-Fi: Pairings with Devices over net-v2, their keys in the OS keychain, finding Devices on
//! the LAN, and the thread that opens a Wi-Fi session for a paired Device with no USB port.
//! USB always wins: a Device that is plugged in links over USB, and unplugging it falls back
//! to Wi-Fi.
use super::{
    Carrier, ControlLayout, DeviceAdded, DeviceId, DeviceSummary, Entry, Inner, KeyMirrorState,
    LinkState, Registry, SetupState, scan,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use td_link::{
    LinkError, Session, SessionConfig,
    net::{self, Address, CompanionId, Credential, NetCarrier, NetError, Refusal, mdns::Advert},
};
use ts_rs::TS;
use zeroize::Zeroize;

/// How long a Wi-Fi connection and its session setup may take.
const CONNECT: Duration = Duration::from_secs(4);
/// How long `hello` may take once the session is up.
const HELLO: Duration = Duration::from_secs(3);
/// The fallback poll of the Wi-Fi thread.
const POLL: Duration = Duration::from_secs(1);
/// The longest wait between tries for a paired Device that isn't answering.
const MAX_RETRY: Duration = Duration::from_secs(10);
/// Maximum unauthenticated busy replies accepted at a discovered trial address.
const TRIAL_BUSY_LIMIT: u8 = 2;
/// A Device that is busy with USB is tried again at most this far apart.
const BUSY_RETRY: Duration = Duration::from_secs(2);
/// A started pairing that nobody finishes is dropped after this.
const ATTEMPT_LIFE: Duration = Duration::from_secs(130);
/// Why Wi-Fi stopped for a Device whose Firmware doesn't speak net-v2 or link-v2. The person
/// updates it over USB; nothing retries on its own.
const NEEDS_FIRMWARE: &str = "The Device's Firmware can't take this Companion over Wi-Fi. Update its Firmware over USB, then connect again.";
/// The service name Pairing keys are saved under in the keychain.
pub const KEY_SERVICE: &str = "touchdeck";

// --- The keychain ---------------------------------------------------------------------------

/// Why a Pairing key couldn't be kept or read. The message says what to do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct KeychainError(pub String);

#[cfg(windows)]
const NO_KEYCHAIN: &str = "Windows Credential Manager isn't available, so the Companion can't keep the Pairing key. Sign in to Windows with your own user account and pair again.";
#[cfg(target_os = "macos")]
const NO_KEYCHAIN: &str = "The macOS keychain isn't available, so the Companion can't keep the Pairing key. Unlock your login keychain and pair again.";
#[cfg(not(any(windows, target_os = "macos")))]
const NO_KEYCHAIN: &str = "No Secret Service keychain answered, so the Companion can't keep the Pairing key. Install and unlock one, such as GNOME Keyring or KWallet, then pair again.";

impl KeychainError {
    pub fn unavailable() -> Self {
        Self(NO_KEYCHAIN.into())
    }
}

/// Where Pairing keys live: the OS keychain, or memory in tests. Never a plain file.
pub trait PairingKeys: Send + Sync + 'static {
    fn save(&self, device: &DeviceId, key: &[u8; 32]) -> Result<(), KeychainError>;
    fn load(&self, device: &DeviceId) -> Result<Option<[u8; 32]>, KeychainError>;
    fn delete(&self, device: &DeviceId) -> Result<(), KeychainError>;
}

/// The OS keychain through `keyring`: Credential Manager on Windows, the Secret Service on
/// Linux. With no keychain every call fails with [`KeychainError::unavailable`].
pub struct OsKeychain;
impl OsKeychain {
    fn entry(device: &DeviceId) -> Result<keyring::Entry, KeychainError> {
        keyring::Entry::new(KEY_SERVICE, &format!("net-pairing:{device}"))
            .map_err(|_| KeychainError::unavailable())
    }
}
fn failed(error: keyring::Error) -> KeychainError {
    KeychainError(format!("{NO_KEYCHAIN} ({error})"))
}
impl PairingKeys for OsKeychain {
    fn save(&self, device: &DeviceId, key: &[u8; 32]) -> Result<(), KeychainError> {
        Self::entry(device)?.set_secret(key).map_err(failed)
    }
    fn load(&self, device: &DeviceId) -> Result<Option<[u8; 32]>, KeychainError> {
        match Self::entry(device)?.get_secret() {
            Ok(mut bytes) => {
                let key = <[u8; 32]>::try_from(bytes.as_slice())
                    .map(Some)
                    .map_err(|_| {
                        KeychainError(
                            "The saved Pairing key is damaged. Forget the Pairing and pair again."
                                .into(),
                        )
                    });
                bytes.zeroize();
                key
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(failed(error)),
        }
    }
    fn delete(&self, device: &DeviceId) -> Result<(), KeychainError> {
        match Self::entry(device)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(failed(error)),
        }
    }
}

/// Keys in memory, for tests and mock Devices. It can act as if there were no keychain.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default, Clone)]
pub struct MemoryKeys {
    keys: Arc<Mutex<BTreeMap<DeviceId, [u8; 32]>>>,
    missing: Arc<std::sync::atomic::AtomicBool>,
}
#[cfg(any(test, feature = "test-hooks"))]
impl MemoryKeys {
    /// While on, every call fails as it does on a PC with no keychain.
    pub fn set_unavailable(&self, missing: bool) {
        self.missing.store(missing, Ordering::SeqCst);
    }
    pub fn has(&self, device: &DeviceId) -> bool {
        self.keys.lock().contains_key(device)
    }
    fn check(&self) -> Result<(), KeychainError> {
        if self.missing.load(Ordering::SeqCst) {
            Err(KeychainError::unavailable())
        } else {
            Ok(())
        }
    }
}
#[cfg(any(test, feature = "test-hooks"))]
impl PairingKeys for MemoryKeys {
    fn save(&self, device: &DeviceId, key: &[u8; 32]) -> Result<(), KeychainError> {
        self.check()?;
        self.keys.lock().insert(device.clone(), *key);
        Ok(())
    }
    fn load(&self, device: &DeviceId) -> Result<Option<[u8; 32]>, KeychainError> {
        self.check()?;
        Ok(self.keys.lock().get(device).copied())
    }
    fn delete(&self, device: &DeviceId) -> Result<(), KeychainError> {
        self.check()?;
        self.keys.lock().remove(device);
        Ok(())
    }
}

// --- Discovery ------------------------------------------------------------------------------

/// Browses the LAN for `_touchdeck._tcp` for at most the given time.
pub type Browse = Arc<dyn Fn(Duration) -> Result<Vec<Advert>, String> + Send + Sync>;

/// Mock discovery: fake net-v2 Devices on loopback announce through it, with the records a
/// real Device sends, so tests discover them without real multicast.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default, Clone)]
pub struct MockNetwork {
    announced: Arc<Mutex<Vec<net::fake::Announcer>>>,
    browsed: Arc<std::sync::atomic::AtomicUsize>,
}
#[cfg(any(test, feature = "test-hooks"))]
impl MockNetwork {
    /// Makes discovery find `device` at its own address. Each browse sees the record it
    /// announces then: hints for its Pairings, and its id only while it is pairing.
    pub fn announce(&self, device: &net::fake::FakeNetDevice) {
        self.announce_at(device, device.addr());
    }
    /// Makes discovery find `device`'s record at `at`, as a Device that moved, or someone
    /// replaying its record elsewhere, would show.
    pub fn announce_at(&self, device: &net::fake::FakeNetDevice, at: std::net::SocketAddr) {
        let mut announced = self.announced.lock();
        announced.retain(|a| a.device_id() != device.device_id());
        announced.push(device.announcer(at));
    }
    /// Takes it off the network, as a network that blocks multicast does.
    pub fn withdraw(&self, device_id: &net::DeviceId) {
        self.announced
            .lock()
            .retain(|a| &a.device_id() != device_id);
    }
    /// How many browses ran.
    pub fn browses(&self) -> usize {
        self.browsed.load(Ordering::SeqCst)
    }
    fn browse(&self) -> Browse {
        let network = self.clone();
        Arc::new(move |_| {
            network.browsed.fetch_add(1, Ordering::SeqCst);
            let records: Vec<_> = network
                .announced
                .lock()
                .iter()
                .filter_map(|a| a.record())
                .collect();
            Ok(net::mdns::merge(
                records.iter().filter_map(net::mdns::read_loopback),
            ))
        })
    }
}

/// How the Device manager reaches Wi-Fi Devices.
pub struct WifiConfig {
    pub keys: Arc<dyn PairingKeys>,
    pub browse: Browse,
    /// `network.json`. `None` keeps Pairings in memory only.
    pub store: Option<PathBuf>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub mock: Option<(MemoryKeys, MockNetwork)>,
}
impl WifiConfig {
    pub fn real(root: &Path) -> Self {
        Self {
            keys: Arc::new(OsKeychain),
            browse: Arc::new(net::mdns::browse),
            store: Some(root.join("network.json")),
            #[cfg(any(test, feature = "test-hooks"))]
            mock: None,
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn mock(store: Option<PathBuf>) -> Self {
        let keys = MemoryKeys::default();
        let network = MockNetwork::default();
        Self {
            keys: Arc::new(keys.clone()),
            browse: network.browse(),
            store,
            mock: Some((keys, network)),
        }
    }
}

// --- network.json ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct SavedPairing {
    device: DeviceId,
    address: String,
    name: Option<String>,
    confirmed: bool,
    paired_ms: u64,
}
#[derive(Serialize, Deserialize)]
struct NetworkFile {
    version: u32,
    companion_id: String,
    pairings: Vec<SavedPairing>,
}

fn load(path: &Path) -> Option<NetworkFile> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<NetworkFile>(&bytes).ok())
        .filter(|file| file.version == 1)
}

fn save(path: &Path, file: &NetworkFile) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(file)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

// --- State ----------------------------------------------------------------------------------

/// A paired Device the Wi-Fi thread tries while it has no USB port.
pub(super) struct WifiRecord {
    pub address: Address,
    pub name: Option<String>,
    pub confirmed: bool,
    pub paired_ms: u64,
    pub due: Instant,
    pub backoff: Duration,
    /// Why the last try failed, for the Pairings list.
    pub problem: Option<String>,
    /// Wi-Fi stopped until the person connects again or plugs the Device in, because its
    /// Firmware can't take the session. Never retried on its own.
    pub stopped: bool,
    /// Where discovery last saw this Pairing's hint, when that isn't `address`. The Wi-Fi
    /// thread tries it next, and it replaces `address` only once a session there works, so a
    /// replayed record only costs a bounded number of failed tries.
    pub trial: Option<Address>,
    pub trial_busy: u8,
    /// `derive(LTK, "discovery")`, read from the keychain once and kept in memory only. It
    /// recognises this Pairing's hints and can't open a session.
    pub discovery: Option<[u8; 32]>,
}

/// A Pairing as the API lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WifiPairing {
    pub device: DeviceId,
    /// Where the Companion reaches the Device: `host:port`.
    pub address: String,
    pub name: Option<String>,
    /// False until the Device confirmed it stored the key, or a session with it worked.
    pub confirmed: bool,
    pub paired_ms: u64,
    /// Why the last Wi-Fi connection failed, or `null`.
    pub problem: Option<String>,
    /// This Companion asked the Device to forget it and hasn't heard back. The key stays until the Device answers
    /// or no longer lists this Companion.
    pub removing: bool,
}

/// A Device found on the LAN.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WifiDevice {
    pub device: DeviceId,
    pub name: String,
    pub address: String,
    /// Pairings the Device says it holds. Only a Device in pairing mode says.
    pub paired: Option<u8>,
    /// Whether this Companion has a Pairing with it.
    pub paired_here: bool,
    /// Whether a pairing window is open on it, so it can pair now. Only then does its record
    /// name it; outside pairing mode only a paired Companion recognises it.
    pub pairing: bool,
}

/// One started pairing, waiting for the code shown on the Device.
struct Attempt {
    pairing: Option<net::Pairing>,
    address: Address,
    device: DeviceId,
    started: Instant,
}

#[derive(Default)]
pub(super) struct WifiState {
    companion: Mutex<Option<CompanionId>>,
    attempts: Mutex<BTreeMap<String, Arc<Mutex<Attempt>>>>,
    /// The `paired` count the last discovery saw at each address.
    advertised: Mutex<BTreeMap<String, Option<u8>>>,
    /// Per Device, the apps whose `net.forget` went unanswered (`pairings.rs`).
    pub(super) forgetting: Mutex<BTreeMap<DeviceId, BTreeSet<CompanionId>>>,
    pub(super) wake: super::Wake,
}

/// A started pairing, as the API returns it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PairingStarted {
    /// The id to send the code to.
    pub attempt: String,
    pub device: DeviceId,
    pub address: String,
    /// Time left in the Device's pairing window.
    pub remaining_ms: u64,
    pub attempts_left: u8,
}

/// Why pairing or a Pairing request failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PairError {
    #[error("The Companion couldn't reach a Device at that address: {0}")]
    Unreachable(String),
    #[error(
        "The Device isn't ready to pair. On the Device, open Settings, choose Pair new app, then try again."
    )]
    WindowClosed,
    #[error("That code is wrong. Check the code on the Device and try again.")]
    WrongCode { attempts_left: u8 },
    #[error(
        "Too many wrong codes. The Device stopped pairing for a while; wait, then open Pair new app on the Device again."
    )]
    Locked,
    /// `full`, with the `paired` count the Device advertised, if discovery saw one.
    #[error("{}", full_message(*.paired))]
    Full { paired: Option<u8> },
    #[error(
        "The Device already has a Pairing with this Companion. Forget it here and on the Device, then pair again."
    )]
    AlreadyPaired,
    #[error("The pairing window on the Device closed. Open Pair new app on the Device again.")]
    Expired,
    #[error(
        "The Device is busy with another app. Close it, or unplug the USB cable, then try again."
    )]
    Busy,
    #[error(
        "The Device's Firmware can't pair over Wi-Fi with this Companion. Update its Firmware over USB, then try again."
    )]
    Version,
    #[error("No pairing with that id is waiting for a code.")]
    AttemptNotFound,
    #[error("This Companion has no Pairing with that Device.")]
    NotPaired,
    #[error("{0}")]
    Keychain(String),
    #[error("Pairing failed: {0}")]
    Failed(String),
}

/// What to tell the person about `full`. Four advertised Pairings means the slots are taken;
/// with no count it may also be this Companion's own Pairing that the Device still holds.
fn full_message(paired: Option<u8>) -> &'static str {
    if paired == Some(FULL) {
        "This Device has four paired apps. Remove one on the Device (paired apps screen), or over USB, then pair again."
    } else {
        "The Device can't take a new Pairing: four apps are paired, or it still has one with this Companion. Remove one on the Device (paired apps screen), or over USB, then pair again."
    }
}

/// The Pairings a Device holds when every slot is taken.
const FULL: u8 = 4;

/// The pairing error for `error`, given the `paired` count the Device last advertised. That
/// count is a hint: it only chooses the advice.
fn pair_refused(error: NetError, paired: Option<u8>) -> PairError {
    match error {
        NetError::Refused(Refusal::Full) => match paired {
            Some(count) if count < FULL => PairError::AlreadyPaired,
            paired => PairError::Full { paired },
        },
        NetError::Refused(Refusal::Closed) if paired == Some(FULL) => PairError::Full { paired },
        error => pair_error(error),
    }
}

fn pair_error(error: NetError) -> PairError {
    match error {
        NetError::Refused(Refusal::Closed) => PairError::WindowClosed,
        NetError::Refused(Refusal::Full) => PairError::Full { paired: None },
        NetError::Refused(Refusal::Expired | Refusal::Cancelled) => PairError::Expired,
        NetError::Refused(Refusal::Busy) => PairError::Busy,
        // No fallback and no retry: net-v1 or plain text would be weaker.
        NetError::Refused(Refusal::Version) | NetError::Version(_) => PairError::Version,
        NetError::Store(detail) => PairError::Keychain(detail),
        NetError::Io(error) => PairError::Unreachable(error.to_string()),
        NetError::Closed => PairError::Expired,
        other => PairError::Failed(other.to_string()),
    }
}

/// The name this Companion shows on the Device: the computer's name.
fn companion_name() -> String {
    let host = std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok());
    name_for(host.as_deref())
}

/// "Open TouchDeck on <host>", with any character net-v2 refuses in a name dropped and cut to
/// 64 bytes on a character boundary.
fn name_for(host: Option<&str>) -> String {
    let host: String = host
        .unwrap_or_default()
        .chars()
        .filter(|&c| net::wire::name_char_allowed(c))
        .collect();
    let mut name = match host.trim() {
        "" => "Open TouchDeck".to_owned(),
        host => format!("Open TouchDeck on {host}"),
    };
    while name.len() > net::wire::NAME_MAX {
        name.pop();
    }
    name.trim_end().to_owned()
}

fn device_id_of(device: &net::DeviceId) -> DeviceId {
    DeviceId::parse(&net::device_hex(device)).expect("12 hex digits are a Device id")
}

impl Inner {
    fn wifi_config(&self) -> &WifiConfig {
        &self.config().wifi
    }

    /// Loads `network.json`, or makes a Companion id. Called once at start.
    pub(super) fn load_wifi(&self, reg: &mut Registry) {
        let file = self.wifi_config().store.as_deref().and_then(load);
        let companion = file
            .as_ref()
            .and_then(|f| net::parse_hex::<16>(&f.companion_id))
            .unwrap_or_else(|| {
                let mut id = [0u8; 16];
                getrandom::fill(&mut id).expect("the OS random source works");
                id
            });
        *self.wifi.companion.lock() = Some(companion);
        let now = Instant::now();
        for saved in file.map(|f| f.pairings).unwrap_or_default() {
            let Some(address) = Address::parse(&saved.address) else {
                continue;
            };
            reg.wifi.insert(
                saved.device,
                WifiRecord {
                    address,
                    name: saved.name,
                    confirmed: saved.confirmed,
                    paired_ms: saved.paired_ms,
                    due: now,
                    backoff: scan::FIRST_RETRY,
                    problem: None,
                    stopped: false,
                    trial: None,
                    trial_busy: 0,
                    discovery: None,
                },
            );
        }
    }

    pub(super) fn companion_id(&self) -> CompanionId {
        let mut companion = self.wifi.companion.lock();
        *companion.get_or_insert_with(|| {
            let mut id = [0u8; 16];
            getrandom::fill(&mut id).expect("the OS random source works");
            id
        })
    }

    /// Every Pairing's discovery key. A key missing from memory is derived from the LTK in the
    /// keychain once; a Pairing whose key can't be read is left out of matching. Blocking.
    fn discovery_keys(&self) -> Vec<(DeviceId, [u8; 32])> {
        let missing: Vec<DeviceId> = {
            let reg = self.reg.lock();
            reg.wifi
                .iter()
                .filter(|(_, r)| r.discovery.is_none())
                .map(|(id, _)| id.clone())
                .collect()
        };
        for device in missing {
            let Ok(Some(mut ltk)) = self.wifi_config().keys.load(&device) else {
                continue;
            };
            let key = net::wire::discovery_key(&ltk);
            ltk.zeroize();
            if let Some(record) = self.reg.lock().wifi.get_mut(&device) {
                record.discovery = Some(key);
            }
        }
        let reg = self.reg.lock();
        reg.wifi
            .iter()
            .filter_map(|(id, r)| r.discovery.map(|key| (id.clone(), key)))
            .collect()
    }

    /// Writes `network.json`. Called off the async runtime.
    fn save_wifi(&self, reg: &Registry) {
        let Some(path) = &self.wifi_config().store else {
            return;
        };
        let file = NetworkFile {
            version: 1,
            companion_id: net::wire::hex(&self.companion_id()),
            pairings: reg
                .wifi
                .iter()
                .map(|(device, r)| SavedPairing {
                    device: device.clone(),
                    address: r.address.to_string(),
                    name: r.name.clone(),
                    confirmed: r.confirmed,
                    paired_ms: r.paired_ms,
                })
                .collect(),
        };
        if let Err(error) = save(path, &file) {
            eprintln!("Companion: couldn't save {}: {error}", path.display());
        }
    }

    /// Makes the Wi-Fi thread try `id` now.
    pub(super) fn wifi_due_now(&self, reg: &mut Registry, id: &DeviceId) {
        if let Some(record) = reg.wifi.get_mut(id) {
            record.due = Instant::now();
            record.backoff = scan::FIRST_RETRY;
            record.stopped = false;
            self.wifi.wake.kick();
        }
    }

    fn wifi_due(&self) -> Vec<(DeviceId, Address)> {
        let reg = self.reg.lock();
        let now = Instant::now();
        reg.wifi
            .iter()
            .filter(|(id, r)| !r.stopped && r.due <= now && wifi_wanted(&reg, id))
            .map(|(id, r)| {
                (
                    id.clone(),
                    r.trial.clone().unwrap_or_else(|| r.address.clone()),
                )
            })
            .collect()
    }

    fn wifi_next(&self) -> Duration {
        let reg = self.reg.lock();
        let now = Instant::now();
        reg.wifi
            .iter()
            .filter(|(id, r)| !r.stopped && wifi_wanted(&reg, id))
            .map(|(_, r)| r.due.saturating_duration_since(now))
            .min()
            .unwrap_or(Duration::MAX)
    }

    fn apply_wifi(self: &Arc<Self>, id: &DeviceId, address: &Address, probed: WifiProbed) {
        let mut reg = self.reg.lock();
        // The scan thread may already have removed the actors to close their links.
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let tried = |r: &WifiRecord| r.trial.as_ref().unwrap_or(&r.address) == address;
        if !wifi_wanted(&reg, id) || reg.wifi.get(id).is_none_or(|r| !tried(r)) {
            drop(reg);
            drop(probed);
            return;
        }
        let record = reg.wifi.get_mut(id).expect("checked above");
        // A discovered address that didn't answer as this Device is dropped, and the Pairing
        // goes back to the address that last worked, at once. Unauthenticated `busy`
        // answers get only a bounded retry at a trial address.
        if record.trial.is_some() && matches!(probed, WifiProbed::Busy) {
            record.trial_busy += 1;
        }
        if record.trial.is_some()
            && (!matches!(probed, WifiProbed::Device(_) | WifiProbed::Busy)
                || record.trial_busy >= TRIAL_BUSY_LIMIT)
        {
            record.trial = None;
            record.due = Instant::now();
            drop(reg);
            drop(probed);
            self.wifi.wake.kick();
            return;
        }
        let found = match probed {
            WifiProbed::Device(found) => found,
            WifiProbed::Busy => {
                record.due = Instant::now() + record.backoff.min(BUSY_RETRY);
                record.backoff = (record.backoff * 2).min(BUSY_RETRY);
                record.problem = Some(PairError::Busy.to_string());
                let link = LinkState::Busy {
                    port: address.to_string(),
                    detail: "The Device is linked over USB or to another app.".into(),
                };
                if !matches!(
                    reg.devices.get(id).map(|e| &e.summary.link),
                    Some(LinkState::Disconnected)
                ) {
                    self.set_link(&mut reg, id, link);
                }
                return;
            }
            WifiProbed::Failed(_) | WifiProbed::Stopped(_) => {
                let (problem, stopped) = match probed {
                    WifiProbed::Failed(problem) => (problem, false),
                    WifiProbed::Stopped(problem) => (problem, true),
                    _ => unreachable!(),
                };
                record.due = Instant::now() + record.backoff;
                record.backoff = (record.backoff * 2).min(MAX_RETRY);
                record.problem = Some(problem);
                record.stopped = stopped;
                let offline = reg.devices.get(id).is_some_and(|e| {
                    matches!(
                        e.summary.link,
                        LinkState::Connecting { .. } | LinkState::Busy { .. }
                    )
                });
                if offline {
                    self.set_link(&mut reg, id, LinkState::Offline);
                }
                return;
            }
        };
        record.backoff = scan::FIRST_RETRY;
        record.problem = None;
        let moved = record.trial.take().is_some();
        if moved {
            record.address = address.clone();
        }
        let newly_confirmed = !record.confirmed;
        record.confirmed = true;
        if found.identity.name.is_some() {
            record.name = found.identity.name.clone();
        }
        let old = self.link_up(
            &mut reg,
            id,
            address.to_string(),
            Carrier::Wifi,
            true,
            *found,
        );
        if newly_confirmed || moved {
            self.save_wifi(&reg);
        }
        drop(reg);
        drop(old);
    }

    /// Records a finished pairing and starts linking over Wi-Fi.
    fn paired(
        &self,
        device: &DeviceId,
        address: &Address,
        confirmed: bool,
        discovery: Option<[u8; 32]>,
    ) {
        let mut reg = self.reg.lock();
        let now = Instant::now();
        let name = reg.devices.get(device).and_then(|e| e.summary.name.clone());
        reg.wifi.insert(
            device.clone(),
            WifiRecord {
                address: address.clone(),
                name,
                confirmed,
                paired_ms: crate::now_ms(),
                due: now,
                backoff: scan::FIRST_RETRY,
                problem: None,
                stopped: false,
                trial: None,
                trial_busy: 0,
                discovery,
            },
        );
        if !reg.devices.contains_key(device) {
            let summary = DeviceSummary {
                id: device.clone(),
                rev: 1,
                name: None,
                setup: SetupState::Ready,
                link: LinkState::Connecting {
                    port: address.to_string(),
                },
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
            self.events.publish(&DeviceAdded(summary.clone()));
            reg.devices.insert(
                device.clone(),
                Entry {
                    summary,
                    actor: None,
                    auto: true,
                    quick_drops: 0,
                    shown: None,
                    reported_page: None,
                },
            );
            self.save(&reg);
        }
        self.save_wifi(&reg);
        drop(reg);
        self.wifi.wake.kick();
    }
}

/// Whether the Wi-Fi thread should try `id`: it's paired, nothing holds it, it has no USB
/// port, and it isn't linked or disconnected by the person.
fn wifi_wanted(reg: &Registry, id: &DeviceId) -> bool {
    reg.wifi.contains_key(id)
        && !reg.updating.contains(id)
        && !reg.ports.values().any(|r| &r.id == id)
        && reg
            .devices
            .get(id)
            .is_none_or(|e| e.auto && e.actor.is_none())
}

enum WifiProbed {
    Device(Box<scan::Found>),
    Busy,
    /// Tried again after a backoff.
    Failed(String),
    /// Not tried again until the person connects or plugs the Device in.
    Stopped(String),
}

fn probe(inner: &Inner, id: &DeviceId, address: &Address) -> WifiProbed {
    let Some(device_id) = net::parse_device_id(id) else {
        return WifiProbed::Failed("The Device id isn't a net-v2 id.".into());
    };
    let mut ltk = match inner.wifi_config().keys.load(id) {
        Ok(Some(key)) => key,
        Ok(None) => {
            return WifiProbed::Failed(
                "The Pairing key is missing from the keychain. Forget the Pairing and pair again."
                    .into(),
            );
        }
        Err(error) => return WifiProbed::Failed(error.0),
    };
    let credential = Credential {
        device_id,
        companion_id: inner.companion_id(),
        ltk,
    };
    ltk.zeroize();
    let connected = NetCarrier::connect(address, &credential, CONNECT);
    drop(credential);
    let carrier = match connected {
        Ok(carrier) => carrier,
        Err(NetError::Refused(Refusal::Busy)) => return WifiProbed::Busy,
        // net-v2 says stop: no weaker fallback and no retry of our own.
        Err(NetError::Refused(Refusal::Version) | NetError::Version(_)) => {
            return WifiProbed::Stopped(NEEDS_FIRMWARE.into());
        }
        Err(NetError::Refused(Refusal::Auth)) => {
            return WifiProbed::Failed(
                "The Device doesn't know this Companion any more. Forget the Pairing and pair again."
                    .into(),
            );
        }
        Err(error) => return WifiProbed::Failed(error.to_string()),
    };
    let config = SessionConfig {
        close_on_restart: true,
        close_on_drop: true,
        ..SessionConfig::default()
    };
    let Ok((session, incoming)) = Session::over(Box::new(carrier), config) else {
        return WifiProbed::Failed("The Wi-Fi session couldn't start.".into());
    };
    let hello = match session.hello(HELLO) {
        Ok(hello) if hello.is_deck() => hello,
        Ok(_) => return WifiProbed::Failed("The Device answered as other Firmware.".into()),
        Err(error) => return WifiProbed::Failed(error.to_string()),
    };
    let identity = match session.identity(HELLO) {
        Ok(identity) => identity,
        Err(LinkError::Timeout(_)) => {
            return WifiProbed::Failed("The Device didn't answer.".into());
        }
        Err(error) => return WifiProbed::Failed(error.to_string()),
    };
    // A net-v2 Device offers link-v2, and its hello carries the id it paired with.
    if identity.protocol != 2 {
        return WifiProbed::Stopped(NEEDS_FIRMWARE.into());
    }
    if identity.id.as_deref().and_then(net::parse_device_id) != Some(device_id) {
        return WifiProbed::Failed("The Device at that address has another id.".into());
    }
    WifiProbed::Device(Box::new(scan::Found {
        session,
        incoming,
        identity,
        hello,
    }))
}

/// The Wi-Fi thread: it wakes on a kick or every second and links paired Devices that are
/// due. It never browses; discovery runs only when the API asks.
pub(super) fn run(inner: Arc<Inner>) {
    while !inner.stop.load(Ordering::SeqCst) {
        for (id, address) in inner.wifi_due() {
            if inner.stop.load(Ordering::SeqCst) {
                break;
            }
            let probed = probe(&inner, &id, &address);
            inner.apply_wifi(&id, &address, probed);
        }
        inner.prune_attempts();
        let next = inner.wifi_next().min(POLL);
        inner.wifi.wake.wait(next);
    }
}

// --- What the API calls ---------------------------------------------------------------------

impl Inner {
    fn prune_attempts(&self) {
        self.wifi.attempts.lock().retain(|_, attempt| {
            attempt
                .try_lock()
                .is_none_or(|a| a.started.elapsed() < ATTEMPT_LIFE)
        });
    }
}

impl super::DeviceManager {
    /// Browses the LAN for up to `wait`. Blocking. A record that carries one of this
    /// Companion's hints is that paired Device; a record in pairing mode is a Device that can
    /// pair now. Every other record is someone else's Device and is dropped.
    pub fn wifi_discover(&self, wait: Duration) -> Result<Vec<WifiDevice>, String> {
        let found = (self.inner.wifi_config().browse)(wait)?;
        let keys = self.inner.discovery_keys();
        *self.inner.wifi.advertised.lock() = found
            .iter()
            .map(|d| {
                (
                    d.address.to_string(),
                    d.pairing.as_ref().and_then(|p| p.paired),
                )
            })
            .collect();
        let mut reg = self.inner.reg.lock();
        let mut by_device = BTreeMap::<DeviceId, WifiDevice>::new();
        let mut moved = false;
        for advert in found {
            let matched: Vec<&DeviceId> = keys
                .iter()
                .filter(|(_, key)| advert.matches(key))
                .map(|(id, _)| id)
                .collect();
            for device in matched {
                // A paired Device that moved, as DHCP may move it, is tried at its new
                // address. The old one stays until a session there works.
                if let Some(record) = reg.wifi.get_mut(device)
                    && record.address != advert.address
                    && record.trial.as_ref() != Some(&advert.address)
                {
                    record.trial = Some(advert.address.clone());
                    record.trial_busy = 0;
                    record.due = Instant::now();
                    record.backoff = scan::FIRST_RETRY;
                    moved = true;
                }
                let name = reg
                    .wifi
                    .get(device)
                    .and_then(|r| r.name.clone())
                    .or_else(|| reg.devices.get(device).and_then(|e| e.summary.name.clone()))
                    .or_else(|| advert.pairing.as_ref().map(|p| p.name.clone()))
                    .unwrap_or_else(|| "TouchDeck".to_owned());
                by_device.insert(
                    device.clone(),
                    WifiDevice {
                        device: device.clone(),
                        name,
                        address: advert.address.to_string(),
                        paired: advert.pairing.as_ref().and_then(|p| p.paired),
                        paired_here: true,
                        pairing: advert.pairing.is_some(),
                    },
                );
            }
            if let Some(mode) = &advert.pairing {
                let device = device_id_of(&mode.device_id);
                by_device
                    .entry(device.clone())
                    .or_insert_with(|| WifiDevice {
                        paired_here: reg.wifi.contains_key(&device),
                        device,
                        name: mode.name.clone(),
                        address: advert.address.to_string(),
                        paired: mode.paired,
                        pairing: true,
                    });
            }
        }
        if moved {
            self.inner.wifi.wake.kick();
        }
        Ok(by_device.into_values().collect())
    }

    /// Every Pairing this Companion holds.
    pub fn wifi_pairings(&self) -> Vec<WifiPairing> {
        let reg = self.inner.reg.lock();
        reg.wifi
            .iter()
            .map(|(device, r)| WifiPairing {
                device: device.clone(),
                address: r.address.to_string(),
                name: r
                    .name
                    .clone()
                    .or_else(|| reg.devices.get(device).and_then(|e| e.summary.name.clone())),
                confirmed: r.confirmed,
                paired_ms: r.paired_ms,
                problem: r.problem.clone(),
                removing: self.inner.removing_here(device),
            })
            .collect()
    }

    /// Asks the Device at `address` to pair. Blocking.
    pub fn wifi_pair_start(&self, address: &str) -> Result<PairingStarted, PairError> {
        let address = Address::parse(address).ok_or_else(|| {
            PairError::Unreachable("That isn't an IP address or host name.".into())
        })?;
        let advertised = self
            .inner
            .wifi
            .advertised
            .lock()
            .get(&address.to_string())
            .copied()
            .flatten();
        let pairing = net::pair(
            &address,
            &self.inner.companion_id(),
            &companion_name(),
            CONNECT,
        )
        .map_err(|error| pair_refused(error, advertised))?;
        let device = device_id_of(&pairing.device_id());
        if self.inner.reg.lock().wifi.contains_key(&device) {
            pairing.cancel();
            return Err(PairError::AlreadyPaired);
        }
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).expect("the OS random source works");
        let attempt = net::wire::hex(&id);
        let started = PairingStarted {
            attempt: attempt.clone(),
            device: device.clone(),
            address: address.to_string(),
            remaining_ms: pairing.time_left().as_millis() as u64,
            attempts_left: pairing.attempts_left(),
        };
        self.inner.prune_attempts();
        self.inner.wifi.attempts.lock().insert(
            attempt,
            Arc::new(Mutex::new(Attempt {
                pairing: Some(pairing),
                address,
                device,
                started: Instant::now(),
            })),
        );
        Ok(started)
    }

    /// Tries the code shown on the Device. Blocking.
    pub fn wifi_pair_code(&self, attempt: &str, code: &str) -> Result<WifiPairing, PairError> {
        let held = self
            .inner
            .wifi
            .attempts
            .lock()
            .get(attempt)
            .cloned()
            .ok_or(PairError::AttemptNotFound)?;
        let mut held = held.lock();
        let Attempt {
            pairing: Some(pairing),
            address,
            device,
            ..
        } = &mut *held
        else {
            return Err(PairError::AttemptNotFound);
        };
        let keys = self.inner.wifi_config().keys.clone();
        let target = device.clone();
        let mut discovery = None;
        let result = pairing.submit(code, &mut |credential| {
            discovery = Some(net::wire::discovery_key(&credential.ltk));
            keys.save(&target, &credential.ltk).map_err(|e| e.0)
        });
        let ended = !pairing.is_open();
        let (address, device) = (address.clone(), device.clone());
        let left = pairing.attempts_left();
        if ended || result.is_ok() {
            held.pairing = None;
            drop(held);
            self.inner.wifi.attempts.lock().remove(attempt);
        }
        match result {
            Ok(paired) => {
                self.inner
                    .paired(&device, &address, paired.confirmed, discovery);
                Ok(self
                    .wifi_pairings()
                    .into_iter()
                    .find(|p| p.device == device)
                    .expect("just paired"))
            }
            Err(NetError::Refused(Refusal::Auth)) if ended || left == 0 => Err(PairError::Locked),
            Err(NetError::Refused(Refusal::Auth)) => Err(PairError::WrongCode {
                attempts_left: left,
            }),
            Err(NetError::Protocol(detail)) if detail.contains("six digits") => {
                Err(PairError::WrongCode {
                    attempts_left: left,
                })
            }
            Err(error) => Err(pair_error(error)),
        }
    }

    /// Gives up a started pairing, which closes the window on the Device.
    pub fn wifi_pair_cancel(&self, attempt: &str) -> Result<(), PairError> {
        let held = self
            .inner
            .wifi
            .attempts
            .lock()
            .remove(attempt)
            .ok_or(PairError::AttemptNotFound)?;
        if let Some(pairing) = held.lock().pairing.take() {
            pairing.cancel();
        }
        Ok(())
    }

    /// Forgets a Pairing: its key leaves the keychain and a Wi-Fi link closes. The Device keeps
    /// its side until someone forgets it there. Blocking.
    pub fn wifi_forget(&self, device: &DeviceId) -> Result<(), PairError> {
        let mut reg = self.inner.reg.lock();
        if !reg.wifi.contains_key(device) {
            return Err(PairError::NotPaired);
        }
        self.inner
            .wifi_config()
            .keys
            .delete(device)
            .map_err(|e| PairError::Keychain(e.0))?;
        reg.wifi.remove(device);
        let mut actor = None;
        if let Some(entry) = reg.devices.get_mut(device)
            && entry
                .actor
                .as_ref()
                .is_some_and(|a| a.carrier == Carrier::Wifi)
        {
            actor = entry.actor.take();
            self.inner.set_link(&mut reg, device, LinkState::Offline);
        } else if reg
            .devices
            .get(device)
            .is_some_and(|e| e.actor.is_none() && !reg.ports.values().any(|r| &r.id == device))
        {
            self.inner.set_link(&mut reg, device, LinkState::Offline);
        }
        self.inner.save_wifi(&reg);
        drop(reg);
        drop(actor);
        Ok(())
    }

    /// Changes where this Companion thinks a paired Device is, as a DHCP move it hasn't seen
    /// yet would. Tests only.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn wifi_move(&self, device: &DeviceId, address: std::net::SocketAddr) {
        if let Some(record) = self.inner.reg.lock().wifi.get_mut(device) {
            record.address = address.into();
            record.trial = None;
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn wifi_mock(&self) -> Option<&(MemoryKeys, MockNetwork)> {
        self.inner.config.get()?.wifi.mock.as_ref()
    }
}

/// A Wi-Fi link that dropped: when to try again.
pub(super) fn relinked(reg: &mut Registry, id: &DeviceId, wait: Duration) {
    if let Some(record) = reg.wifi.get_mut(id) {
        record.due = Instant::now() + wait;
        record.backoff = scan::FIRST_RETRY;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_json_round_trips_and_a_bad_file_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("network.json");
        assert!(load(&path).is_none());
        let file = NetworkFile {
            version: 1,
            companion_id: "101112131415161718191a1b1c1d1e1f".into(),
            pairings: vec![SavedPairing {
                device: DeviceId::parse("020000000001").unwrap(),
                address: "192.0.2.10:47820".into(),
                name: Some("Desk".into()),
                confirmed: true,
                paired_ms: 5,
            }],
        };
        save(&path, &file).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.companion_id, file.companion_id);
        assert_eq!(loaded.pairings[0].address, "192.0.2.10:47820");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("ltk") && !text.contains("key"), "{text}");
        std::fs::write(&path, br#"{"version":2}"#).unwrap();
        assert!(load(&path).is_none());
    }

    #[test]
    fn memory_keys_act_like_a_missing_keychain_on_request() {
        let keys = MemoryKeys::default();
        let id = DeviceId::parse("020000000001").unwrap();
        keys.save(&id, &[7; 32]).unwrap();
        assert_eq!(keys.load(&id).unwrap(), Some([7; 32]));
        keys.set_unavailable(true);
        assert_eq!(keys.load(&id), Err(KeychainError::unavailable()));
        assert!(KeychainError::unavailable().0.contains("pair again"));
        keys.set_unavailable(false);
        keys.delete(&id).unwrap();
        assert_eq!(keys.load(&id).unwrap(), None);
    }

    #[test]
    fn full_and_closed_take_their_advice_from_the_advertised_count() {
        let full = || NetError::Refused(Refusal::Full);
        let closed = || NetError::Refused(Refusal::Closed);
        assert_eq!(
            pair_refused(full(), Some(4)),
            PairError::Full { paired: Some(4) }
        );
        assert_eq!(pair_refused(full(), Some(2)), PairError::AlreadyPaired);
        assert_eq!(pair_refused(full(), None), PairError::Full { paired: None });
        assert_eq!(
            pair_refused(closed(), Some(4)),
            PairError::Full { paired: Some(4) }
        );
        assert_eq!(pair_refused(closed(), Some(3)), PairError::WindowClosed);
        assert!(
            PairError::Full { paired: Some(4) }
                .to_string()
                .contains("four paired apps")
        );
        assert!(
            PairError::Full { paired: None }
                .to_string()
                .contains("still has one with this Companion")
        );
        for version in [NetError::Refused(Refusal::Version), NetError::Version(1)] {
            assert_eq!(pair_refused(version, None), PairError::Version);
        }
    }

    #[test]
    fn the_companion_name_is_valid_on_the_wire() {
        assert!(net::wire::valid_name(&companion_name()));
        assert_eq!(name_for(Some("DESK-PC\n")), "Open TouchDeck on DESK-PC");
        assert_eq!(
            name_for(Some("zero\u{200b}width\u{3164}")),
            "Open TouchDeck on zerowidth"
        );
        assert_eq!(name_for(None), "Open TouchDeck");
        assert_eq!(name_for(Some("\u{feff}")), "Open TouchDeck");
        let long = name_for(Some(&"\u{e9}".repeat(40)));
        assert!(
            long.len() <= net::wire::NAME_MAX && net::wire::valid_name(&long),
            "{long}"
        );
    }
}
