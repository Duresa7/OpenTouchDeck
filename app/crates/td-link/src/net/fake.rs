//! A net-v2 Device on loopback for tests. It listens on `127.0.0.1`, pairs and opens sessions
//! as `net-v2.md` says, and runs each session's link frames through a [`MockDeck`], so the
//! Companion sees the same Device over Wi-Fi that `MockPorts` shows over USB. The test plays
//! the person at the Device: it opens the pairing window, reads the code and taps Install.
//!
//! It announces itself through [`FakeNetDevice::advert_at`], the record a real Device sends,
//! so tests discover it without real multicast.
//!
//! What it leaves out: per-peer backoff (every test peer is 127.0.0.1), takeover of an
//! unproven owner, and BLE.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use td_core::{Body, Frame, Item, Role, StreamDecoder};
use x25519_dalek::{EphemeralSecret, PublicKey};

use super::socket::Items;
use super::wire::{
    self, C2D, D2C, PAIR_ACCEPT, PAIR_CHALLENGE, PAIR_COMMIT, PAIR_OFFER, PAIR_PROOF, PAIR_REQUEST,
    PAIR_START, PAIR_STORED, PairKeys, Phase, RecordReceiver, RecordSender, SESSION_CHALLENGE,
    SESSION_OPEN, SESSION_PROOF, SESSION_READY, SessionKeys, SrpServer,
};
use super::{CompanionId, DeviceId, NetError, Refusal, mdns, random, random_exponent};
use crate::{MockDeck, Transport};

const FIRST_ITEM: Duration = Duration::from_secs(2);
const SETUP: Duration = Duration::from_secs(10);
const ATTEMPT: Duration = Duration::from_secs(10);
const WINDOW: Duration = Duration::from_millis(wire::PAIR_WINDOW_MS as u64);
const PROMPT: Duration = Duration::from_secs(60);
const COOLDOWN: Duration = Duration::from_secs(60);
const COOLDOWN_MAX: Duration = Duration::from_secs(3_600);
const EXPIRY: Duration = Duration::from_secs(5);
const MAX_PAIRINGS: usize = 4;

/// What net-v2 lets a network session ask. `update.begin` is confirmed on the Device;
/// everything else is USB-only.
pub const NETWORK_MESSAGES: &[&str] = &[
    "hello",
    "ping",
    "link.state",
    "layout.get",
    "layout.put",
    "icon.put",
    "icon.list",
    "icon.rm",
    "settings.get",
    "settings.put",
    "store.stat",
    "xfer.end",
    "widget.set",
    "dashboard.set",
    "v2.name.get",
    "v2.name.set",
    "v2.asset.stat",
    "v2.asset.list",
    "v2.asset.put",
    "v2.asset.end",
    "v2.asset.delete",
    "v2.live.set",
    "v2.photo.put",
    "v2.photo.end",
    "v2.idle.get",
    "v2.idle.set",
    "v2.clock.set",
    "net.state",
];

struct Stored {
    name: String,
    ltk: [u8; 32],
}

struct Window {
    id: [u8; 16],
    ends: Instant,
    code: Option<String>,
    failures: u8,
    bound: bool,
}

enum Prompt {
    None,
    Showing {
        bytes: u64,
        sha: String,
        approved: bool,
        until: Instant,
    },
    Ended {
        bytes: u64,
        sha: String,
    },
}

#[derive(Default)]
struct State {
    pairings: BTreeMap<CompanionId, Stored>,
    window: Option<Window>,
    pair_failures: u32,
    locked_until: Option<Instant>,
    owner: Option<(u64, TcpStream)>,
    session: Option<String>,
    /// The Pairing of the network session, while there is one.
    peer: Option<CompanionId>,
    prompt: Option<Prompt>,
    /// Per Pairing: when its update prompt cooldown ends, and how long the last one was.
    cooldowns: BTreeMap<CompanionId, (Instant, Duration)>,
    sessions: u32,
    /// Where the next session's record counters start, for tests of the record limit.
    skip: Option<u64>,
    /// An error item every `session.open` gets, as a Device that can't take the session.
    session_refusal: Option<Refusal>,
    unreachable: bool,
    lose_stored: bool,
}

struct Inner {
    deck: MockDeck,
    device_id: DeviceId,
    addr: SocketAddr,
    state: Mutex<State>,
    stopped: AtomicBool,
    connections: AtomicU64,
}

/// A fake net-v2 Device. Dropping it stops the listener.
pub struct FakeNetDevice {
    inner: Arc<Inner>,
}

impl FakeNetDevice {
    /// Listens on an ephemeral loopback port in front of `deck`.
    pub fn start(deck: MockDeck) -> FakeNetDevice {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let inner = Arc::new(Inner {
            device_id: deck.device_id(),
            deck,
            addr: listener.local_addr().expect("a bound address"),
            state: Mutex::new(State::default()),
            stopped: AtomicBool::new(false),
            connections: AtomicU64::new(0),
        });
        let accepting = inner.clone();
        thread::Builder::new()
            .name("fake-net-device".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if accepting.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let device = accepting.clone();
                    thread::spawn(move || device.serve(stream));
                }
            })
            .expect("the fake Device listens");
        FakeNetDevice { inner }
    }

    pub fn addr(&self) -> SocketAddr {
        self.inner.addr
    }

    pub fn device_id(&self) -> DeviceId {
        self.inner.device_id
    }

    pub fn deck(&self) -> &MockDeck {
        &self.inner.deck
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state()
    }

    /// The person chooses "Pair new app" on the Device. Fails while pairing is locked out.
    pub fn open_window(&self) -> Result<(), Duration> {
        let mut state = self.state();
        if let Some(left) = locked(&state) {
            return Err(left);
        }
        let mut id = [0u8; 16];
        random(&mut id);
        state.window = Some(Window {
            id,
            ends: Instant::now() + WINDOW,
            code: None,
            failures: 0,
            bound: false,
        });
        Ok(())
    }

    /// The DNS-SD record the Device announces now, as if it were at `at`. In pairing mode it
    /// carries the id, name and Pairing count. Otherwise it names nothing: a fresh nonce and four
    /// hints, one per stored Pairing in slot order and a random filler per free slot.
    pub fn advert_at(&self, at: SocketAddr) -> mdns::Resolved {
        self.inner.advert(at)
    }

    /// Announces the Device at `at` for as long as it runs: each call gives its current record,
    /// and `None` once the Device is dropped. Mock discovery browses through these.
    pub fn announcer(&self, at: SocketAddr) -> Announcer {
        Announcer {
            device: Arc::downgrade(&self.inner),
            device_id: self.inner.device_id,
            at,
        }
    }

    /// The person taps Cancel on the pairing screen.
    pub fn cancel_window(&self) {
        self.state().window = None;
    }

    /// The code on the Device's screen, once a Companion has asked to pair.
    pub fn code(&self) -> Option<String> {
        self.state().window.as_ref().and_then(|w| w.code.clone())
    }

    /// Whether a pairing window is open.
    pub fn pairing(&self) -> bool {
        self.state()
            .window
            .as_ref()
            .is_some_and(|w| w.ends > Instant::now())
    }

    /// Stored Pairings.
    pub fn paired(&self) -> usize {
        self.state().pairings.len()
    }

    /// The names of stored Pairings.
    pub fn paired_names(&self) -> Vec<String> {
        self.state()
            .pairings
            .values()
            .map(|p| p.name.clone())
            .collect()
    }

    /// The stored failed-code counter.
    pub fn pair_failures(&self) -> u32 {
        self.state().pair_failures
    }

    /// Time left in a pairing lockout, zero when none.
    pub fn locked_for(&self) -> Duration {
        locked(&self.state()).unwrap_or_default()
    }

    /// Ends the lockout, as if its time ran out.
    pub fn end_lockout(&self) {
        self.state().locked_until = None;
    }

    /// `net.forget` over USB: every Pairing, the counter and the lockout go.
    pub fn forget_all(&self) {
        let mut state = self.state();
        state.pairings.clear();
        state.pair_failures = 0;
        state.locked_until = None;
        state.window = None;
        if let Some((_, stream)) = state.owner.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    /// The Companion name of the running session.
    pub fn session(&self) -> Option<String> {
        self.state().session.clone()
    }

    /// Sessions that reached `session.ready`.
    pub fn sessions(&self) -> u32 {
        self.state().sessions
    }

    /// Ends the running carrier connection, as a dropped Wi-Fi link does.
    pub fn drop_connection(&self) {
        if let Some((_, stream)) = self.state().owner.as_ref() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    /// While on, the Device drops every new connection at once, as if it had left the network.
    pub fn set_unreachable(&self, unreachable: bool) {
        self.state().unreachable = unreachable;
        if unreachable {
            self.drop_connection();
        }
    }

    /// Commits the next Pairing but never sends `pair.stored`.
    pub fn lose_stored(&self) {
        self.state().lose_stored = true;
    }

    /// The Firmware update prompt on screen: image size and SHA-256.
    pub fn update_prompt(&self) -> Option<(u64, String)> {
        match self.state().prompt.as_ref() {
            Some(Prompt::Showing {
                bytes, sha, until, ..
            }) if *until > Instant::now() => Some((*bytes, sha.clone())),
            _ => None,
        }
    }

    /// The person taps Install on the update prompt. That also ends the cooldown.
    pub fn tap_install(&self) -> bool {
        let mut state = self.state();
        let State {
            prompt,
            peer,
            cooldowns,
            ..
        } = &mut *state;
        match prompt.as_mut() {
            Some(Prompt::Showing { approved, .. }) => {
                *approved = true;
                if let Some(peer) = peer {
                    cooldowns.remove(peer);
                }
                true
            }
            _ => false,
        }
    }

    /// The person taps Cancel on the update prompt.
    pub fn tap_cancel(&self) {
        end_prompt(&mut self.state(), Instant::now());
    }

    /// Time left in the session's update prompt cooldown.
    /// Answers every `session.open` with `refusal`, or runs sessions again with `None`.
    pub fn refuse_sessions_with(&self, refusal: Option<Refusal>) {
        self.inner.state().session_refusal = refusal;
    }

    /// How many connections reached the Device.
    pub fn connections(&self) -> u64 {
        self.inner.connections.load(Ordering::Acquire)
    }

    /// Starts the next session's record counters at `counter` in both directions, as
    /// [`NetCarrier::skip_records`](super::NetCarrier::skip_records) does on the Companion.
    pub fn skip_next_session(&self, counter: u64) {
        self.inner.state().skip = Some(counter);
    }

    pub fn update_cooldown(&self) -> Option<Duration> {
        let state = self.state();
        let (until, _) = state.cooldowns.get(state.peer.as_ref()?)?;
        Some(until.saturating_duration_since(Instant::now())).filter(|left| !left.is_zero())
    }

    /// The session's update prompt cooldown ends now; the next one is still longer.
    pub fn end_update_cooldown(&self) {
        let mut state = self.state();
        let Some(peer) = state.peer else { return };
        if let Some((until, _)) = state.cooldowns.get_mut(&peer) {
            *until = Instant::now();
        }
    }
}

impl Drop for FakeNetDevice {
    fn drop(&mut self) {
        self.inner.stopped.store(true, Ordering::Release);
        self.drop_connection();
        // Wakes the accept loop so it sees the stop.
        let _ = TcpStream::connect_timeout(&self.inner.addr, Duration::from_millis(200));
    }
}

/// Counts a failed pairing in the stored counter; from the third on, pairing locks out.
fn count_failure(state: &mut State) {
    state.pair_failures += 1;
    if state.pair_failures >= 3 {
        let lock =
            Duration::from_millis((30_000u64 << (state.pair_failures - 3).min(5)).min(900_000));
        state.locked_until = Some(Instant::now() + lock);
    }
}

/// Ends a prompt that wasn't approved, at `at`, and starts or lengthens its Pairing's
/// cooldown: 60 s, doubling up to an hour.
fn end_prompt(state: &mut State, at: Instant) {
    let Some(Prompt::Showing {
        bytes,
        sha,
        approved: false,
        ..
    }) = state.prompt.take_if(|p| {
        matches!(
            p,
            Prompt::Showing {
                approved: false,
                ..
            }
        )
    })
    else {
        return;
    };
    state.prompt = Some(Prompt::Ended { bytes, sha });
    if let Some(peer) = state.peer {
        let length = state
            .cooldowns
            .get(&peer)
            .map_or(COOLDOWN, |(_, last)| (*last * 2).min(COOLDOWN_MAX));
        state.cooldowns.insert(peer, (at + length, length));
    }
}

fn locked(state: &State) -> Option<Duration> {
    state
        .locked_until
        .map(|until| until.saturating_duration_since(Instant::now()))
        .filter(|left| !left.is_zero())
}

/// Clears ownership when a carrier connection ends.
struct Owned<'a>(&'a Inner, u64);

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state();
        if state.owner.as_ref().is_some_and(|(id, _)| *id == self.1) {
            state.owner = None;
            state.session = None;
            // A window dropped before a commit counts as a failed code, unless it expired;
            // Cancel on the Device and a counted proof already cleared it.
            if let Some(window) = state.window.take_if(|w| w.bound)
                && window.ends > Instant::now()
            {
                count_failure(&mut state);
            }
            end_prompt(&mut state, Instant::now());
            if let Some(Prompt::Showing { bytes, sha, .. }) = state.prompt.take() {
                state.prompt = Some(Prompt::Ended { bytes, sha });
            }
            state.peer = None;
        }
    }
}

fn refuse(items: &mut Items, reason: Refusal) {
    let _ = items.send(
        &wire::error_item(reason),
        Instant::now() + Duration::from_millis(100),
    );
    items.finish();
}

/// A fake Device's announcement, as [`FakeNetDevice::announcer`] makes it.
#[derive(Clone)]
pub struct Announcer {
    device: std::sync::Weak<Inner>,
    device_id: DeviceId,
    at: SocketAddr,
}

impl Announcer {
    /// The Device this announces. Tests use it to withdraw the announcement.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// The record a browse sees now, or `None` once the Device is gone.
    pub fn record(&self) -> Option<mdns::Resolved> {
        let device = self.device.upgrade()?;
        (!device.stopped.load(Ordering::Acquire)).then(|| device.advert(self.at))
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn advert(&self, at: SocketAddr) -> mdns::Resolved {
        let mut nonce = [0u8; wire::ADVERT_NONCE_LEN];
        random(&mut nonce);
        let state = self.state();
        let mut hints = String::new();
        for pairing in state.pairings.values() {
            hints += &wire::hex(&wire::hint(&wire::discovery_key(&pairing.ltk), &nonce));
        }
        for _ in state.pairings.len()..MAX_PAIRINGS {
            let mut filler = [0u8; wire::HINT_LEN];
            random(&mut filler);
            hints += &wire::hex(&filler);
        }
        let mut txt = BTreeMap::from([
            ("proto".to_owned(), "2".to_owned()),
            ("n".to_owned(), wire::hex(&nonce)),
            ("h".to_owned(), hints),
        ]);
        let pairing = locked(&state).is_none()
            && state
                .window
                .as_ref()
                .is_some_and(|w| w.ends > Instant::now());
        if pairing {
            txt.insert("id".into(), wire::hex(&self.device_id));
            txt.insert("name".into(), self.deck.device_name());
            txt.insert("paired".into(), state.pairings.len().to_string());
        }
        mdns::Resolved {
            addresses: vec![at.ip()],
            port: at.port(),
            txt,
        }
    }

    fn serve(&self, stream: TcpStream) {
        if self.stopped.load(Ordering::Acquire) || self.state().unreachable {
            return;
        }
        let mut items = Items::new(stream);
        let conn = self.connections.fetch_add(1, Ordering::AcqRel);
        {
            let mut state = self.state();
            if state.owner.is_some() {
                drop(state);
                refuse(&mut items, Refusal::Busy);
                return;
            }
            let Ok(clone) = items.stream.try_clone() else {
                return;
            };
            state.owner = Some((conn, clone));
        }
        let _owned = Owned(self, conn);
        let started = Instant::now();
        let first = match items.read(Phase::Handshake, started + FIRST_ITEM) {
            Ok(first) => first,
            Err(error) if error.is_timeout() => return refuse(&mut items, Refusal::Expired),
            Err(_) => return items.close(),
        };
        match wire::parse_handshake(&first) {
            Ok((PAIR_REQUEST, fields)) => {
                let fields = fields.to_vec();
                self.pair(&mut items, &first, &fields);
            }
            Ok((SESSION_OPEN, fields)) => {
                if let Some(refusal) = self.state().session_refusal {
                    return refuse(&mut items, refusal);
                }
                let fields = fields.to_vec();
                self.session(items, &fields, started);
            }
            Err(NetError::Version(_)) => refuse(&mut items, Refusal::Version),
            _ => items.close(),
        }
    }

    fn pair(&self, items: &mut Items, request: &[u8], fields: &[u8]) {
        let companion_id: CompanionId = fields[..16].try_into().expect("checked length");
        let name = String::from_utf8_lossy(&fields[17..]).into_owned();
        let (window_id, ends, code) = {
            let mut state = self.state();
            let now = Instant::now();
            if locked(&state).is_some() || state.window.as_ref().is_none_or(|w| w.ends <= now) {
                state.window = None;
                drop(state);
                return refuse(items, Refusal::Closed);
            }
            if state.pairings.len() >= MAX_PAIRINGS || state.pairings.contains_key(&companion_id) {
                drop(state);
                return refuse(items, Refusal::Full);
            }
            let window = state.window.as_mut().expect("checked above");
            let code = format!("{:06}", pairing_code());
            window.bound = true;
            window.code = Some(code.clone());
            (window.id, window.ends, code)
        };
        let left = ends.saturating_duration_since(Instant::now()).as_millis() as u32;
        let Ok(offer) = wire::handshake(
            PAIR_OFFER,
            &[&self.device_id[..], &window_id, &left.max(1).to_be_bytes()].concat(),
        ) else {
            return items.close();
        };
        if items.send(&offer, ends).is_err() {
            return;
        }
        loop {
            // Waits for the person to type the code, noticing Cancel and expiry.
            let start = loop {
                let open = self.state().window.as_ref().map(|w| w.ends);
                match open {
                    None => return refuse(items, Refusal::Cancelled),
                    Some(ends) if ends <= Instant::now() => {
                        self.state().window = None;
                        return refuse(items, Refusal::Expired);
                    }
                    Some(_) => {}
                }
                match items.poll(Phase::Handshake, Duration::from_millis(50)) {
                    Ok(Some(item)) => break item,
                    Ok(None) => {}
                    Err(_) => return items.close(),
                }
            };
            if !matches!(wire::parse_handshake(&start), Ok((PAIR_START, _))) {
                return items.close();
            }
            if self.state().window.is_none() {
                return refuse(items, Refusal::Cancelled);
            }
            let deadline = Instant::now() + ATTEMPT;
            let a_pub: [u8; 384] = start[4..388].try_into().expect("checked length");
            let mut salt = [0u8; 16];
            random(&mut salt);
            let mut nonce = [0u8; 32];
            random(&mut nonce);
            let identity = wire::identity(&self.device_id, &companion_id);
            let Ok(server) = SrpServer::new(&identity, &code, &salt, &random_exponent()) else {
                return items.close();
            };
            let Ok(challenge) =
                wire::handshake(PAIR_CHALLENGE, &[&salt[..], &server.b_pub, &nonce].concat())
            else {
                return items.close();
            };
            let Ok(key) = server.key(&a_pub) else {
                return items.close();
            };
            let Ok(keys) = PairKeys::derive(&key, request, &offer, &start, &challenge) else {
                return items.close();
            };
            if items.send(&challenge, deadline).is_err() {
                return;
            }
            let Ok((_, proof)) = items.expect(PAIR_PROOF, deadline) else {
                return items.close();
            };
            if !wire::same(&proof, &keys.client_proof) {
                let end = {
                    let mut state = self.state();
                    count_failure(&mut state);
                    let failures = state.window.as_mut().map_or(u8::MAX, |w| {
                        w.failures += 1;
                        w.failures
                    });
                    let end = state.pair_failures >= 3 || failures >= 3;
                    if end {
                        state.window = None;
                    }
                    end
                };
                if end {
                    return refuse(items, Refusal::Auth);
                }
                if items
                    .send(&wire::error_item(Refusal::Auth), deadline)
                    .is_err()
                {
                    return;
                }
                continue;
            }
            let mut ltk = [0u8; 32];
            random(&mut ltk);
            let accept = [&keys.deck_proof[..], &keys.wrap_key(&ltk)].concat();
            let Ok(accept) = wire::handshake(PAIR_ACCEPT, &accept) else {
                return items.close();
            };
            if items.send(&accept, deadline).is_err() {
                return;
            }
            let Ok((_, commit)) = items.expect(PAIR_COMMIT, deadline) else {
                return items.close();
            };
            if !wire::same(&commit, &keys.commit_proof(&ltk)) {
                return refuse(items, Refusal::Auth);
            }
            let lose = {
                let mut state = self.state();
                state.pairings.insert(companion_id, Stored { name, ltk });
                state.pair_failures = 0;
                state.locked_until = None;
                state.window = None;
                std::mem::take(&mut state.lose_stored)
            };
            if !lose && let Ok(stored) = wire::handshake(PAIR_STORED, &keys.stored_proof(&ltk)) {
                let _ = items.send(&stored, deadline);
            }
            return items.finish();
        }
    }

    fn session(&self, mut items: Items, fields: &[u8], started: Instant) {
        let deadline = started + SETUP;
        let tag: [u8; wire::OPEN_TAG_LEN] = fields[..16].try_into().expect("checked length");
        let client_share: [u8; 32] = fields[16..48].try_into().expect("checked length");
        let device_id = self.device_id;
        // Compute every slot, including empty ones, as the Device does.
        let mut found = None;
        let state = self.state();
        let mut entries = state.pairings.iter();
        for _ in 0..MAX_PAIRINGS {
            let entry = entries.next();
            let key = entry.map_or(&[0; 32], |(_, pairing)| &pairing.ltk);
            let matches = wire::same(&wire::open_tag(key, &client_share), &tag);
            let Some((id, pairing)) = entry else { continue };
            if matches && found.is_none() {
                found = Some((*id, pairing.name.clone(), pairing.ltk));
            }
        }
        drop(state);
        let Some((companion_id, name, ltk)) = found else {
            return refuse(&mut items, Refusal::Auth);
        };
        if self.deck.usb_online() {
            return refuse(&mut items, Refusal::Busy);
        }
        let secret = EphemeralSecret::random();
        let device_share = PublicKey::from(&secret).to_bytes();
        let Ok(shared) = wire::agree(secret, &client_share) else {
            return items.close();
        };
        let Ok(keys) = SessionKeys::derive(
            &ltk,
            &device_id,
            &companion_id,
            &client_share,
            &device_share,
            shared.as_bytes(),
        ) else {
            return items.close();
        };
        let Ok(challenge) = wire::handshake(
            SESSION_CHALLENGE,
            &[&device_share[..], &keys.deck_proof].concat(),
        ) else {
            return items.close();
        };
        if items.send(&challenge, deadline).is_err() {
            return;
        }
        let Ok((_, proof)) = items.expect(SESSION_PROOF, deadline) else {
            return items.close();
        };
        if !wire::same(&proof, &keys.client_proof) {
            return items.close();
        }
        let Ok(ready) = wire::handshake(SESSION_READY, &keys.ready_proof) else {
            return items.close();
        };
        if items.send(&ready, deadline).is_err() {
            return;
        }
        let skip = {
            let mut state = self.state();
            state.session = Some(name);
            state.peer = Some(companion_id);
            state.sessions += 1;
            state.skip.take().unwrap_or(0)
        };
        let mut sender = RecordSender::new(&keys.d2c, D2C);
        let mut receiver = RecordReceiver::new(&keys.c2d, C2D);
        sender.skip_to(skip);
        receiver.skip_to(skip);
        self.bridge(items, sender, receiver);
    }

    /// Runs the session's link frames through the mock: records in, SLIP frames to the mock,
    /// the mock's frames back out as records. Console text never crosses.
    fn bridge(&self, mut items: Items, sender: RecordSender, mut receiver: RecordReceiver) {
        let Ok(stream) = items.stream.try_clone() else {
            return;
        };
        let Ok((mut from_deck, mut to_deck)) = Box::new(self.deck.connect_network()).split() else {
            return;
        };
        let out = Arc::new(Mutex::new((stream, sender)));
        let alive = Arc::new(AtomicBool::new(true));
        let pump = {
            let out = out.clone();
            let alive = alive.clone();
            thread::spawn(move || {
                let mut decoder = StreamDecoder::default();
                let mut buf = [0u8; 4096];
                while alive.load(Ordering::Acquire) {
                    let bytes = match from_deck.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => &buf[..n],
                        Err(e) if e.kind() == ErrorKind::TimedOut => &[][..],
                        Err(_) => break,
                    };
                    let frames = decoder.feed(bytes, Instant::now());
                    for item in frames {
                        if let Item::Frame(frame) = item
                            && send(&out, &frame).is_err()
                        {
                            alive.store(false, Ordering::Release);
                        }
                    }
                }
                alive.store(false, Ordering::Release);
                let _ = out.lock().unwrap().0.shutdown(Shutdown::Both);
            })
        };
        let mut last = Instant::now();
        while alive.load(Ordering::Acquire) {
            let item = match items.poll(Phase::Record, crate::READ_TICK) {
                Ok(Some(item)) => item,
                Ok(None) if last.elapsed() < EXPIRY => continue,
                _ => break,
            };
            let Ok(body) = receiver.open(&item) else {
                break;
            };
            let Ok(frame) = Frame::decode_body(&body) else {
                break;
            };
            last = Instant::now();
            match self.filter(&frame) {
                Some(reply) => {
                    if send(&out, &reply).is_err() {
                        break;
                    }
                }
                None => {
                    let Ok(wire) = frame.encode() else { break };
                    if to_deck.write_all(&wire).is_err() {
                        break;
                    }
                }
            }
        }
        alive.store(false, Ordering::Release);
        items.close();
        drop(to_deck);
        let _ = pump.join();
    }

    /// Answers what the Device enforces itself on a network session: USB-only requests and
    /// the Firmware update prompt. `None` passes the frame to the mock.
    fn filter(&self, frame: &Frame) -> Option<Frame> {
        let (Role::Request(seq), Body::Message(payload)) = (frame.role, &frame.body) else {
            return None;
        };
        let request: Value = serde_json::from_slice(payload).ok()?;
        let m = request.get("m")?.as_str()?.to_owned();
        let refuse = |reason: &str, error: &str| {
            Some(Frame::message(
                Role::Response(seq),
                json!({"m": m, "ok": false, "reason": reason, "error": error})
                    .to_string()
                    .into_bytes(),
            ))
        };
        if m == "update.begin" {
            let bytes = request.get("bytes").and_then(Value::as_u64).unwrap_or(0);
            let sha = request
                .get("sha256")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let mut state = self.state();
            let now = Instant::now();
            if let Some(Prompt::Showing { until, .. }) = state.prompt
                && until <= now
            {
                end_prompt(&mut state, until);
            }
            let cooling = state
                .peer
                .and_then(|peer| state.cooldowns.get(&peer))
                .is_some_and(|(until, _)| *until > now);
            if cooling {
                // That is also the repeat an ended prompt owes `declined`.
                if matches!(state.prompt, Some(Prompt::Ended { .. })) {
                    state.prompt = None;
                }
                return refuse("declined", "The update was declined on the Device");
            }
            let prompt = state.prompt.take();
            let (next, answer) = match prompt {
                Some(Prompt::Showing {
                    bytes: b,
                    sha: s,
                    approved,
                    until,
                }) if b == bytes && s == sha => {
                    if approved {
                        (Prompt::None, None)
                    } else {
                        (
                            Prompt::Showing {
                                bytes,
                                sha,
                                approved,
                                until,
                            },
                            Some("confirm"),
                        )
                    }
                }
                Some(Prompt::Showing {
                    bytes, sha, until, ..
                }) => {
                    // A different image ends the prompt unapproved.
                    state.prompt = Some(Prompt::Showing {
                        bytes,
                        sha,
                        approved: false,
                        until,
                    });
                    end_prompt(&mut state, now);
                    return refuse("declined", "The update was declined on the Device");
                }
                Some(Prompt::Ended { bytes: b, sha: s }) if b == bytes && s == sha => {
                    (Prompt::None, Some("declined"))
                }
                _ => (
                    Prompt::Showing {
                        bytes,
                        sha,
                        approved: false,
                        until: now + PROMPT,
                    },
                    Some("confirm"),
                ),
            };
            state.prompt = Some(next);
            return match answer {
                None => None,
                Some("confirm") => refuse("confirm", "Confirm the update on the Device"),
                Some(_) => refuse("declined", "The update was declined on the Device"),
            };
        }
        if NETWORK_MESSAGES.contains(&m.as_str()) {
            None
        } else {
            refuse("usb-only", "Only over USB")
        }
    }
}

/// A uniform six-digit code: values at or above the last whole million are drawn again, so
/// no code is likelier than another.
fn pairing_code() -> u32 {
    const BOUND: u32 = u32::MAX - (u32::MAX % 1_000_000) - 1;
    loop {
        let mut raw = [0u8; 4];
        random(&mut raw);
        let value = u32::from_be_bytes(raw);
        if value <= BOUND {
            return value % 1_000_000;
        }
    }
}

fn send(out: &Mutex<(TcpStream, RecordSender)>, frame: &Frame) -> Result<(), ()> {
    let body = frame.encode_body().map_err(|_| ())?;
    let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
    let item = out.1.seal(&body).map_err(|_| ())?;
    out.0.set_write_timeout(Some(Duration::from_secs(2))).ok();
    out.0.write_all(&item).map_err(|_| ())
}
