//! The mock Device's Pairings: the radio build's `net.state`, `net.pair`, `net.list` and `net.forget` as
//! `schema/protocol/net-v2.md` defines them. No network is involved.
//!
//! The Pairings live in a [`NetStore`]. A bare `MockDeck` keeps its own list, which a test or the demo fills with
//! [`MockDeck::add_net_pairing`](crate::MockDeck::add_net_pairing). The fake net-v2 Device puts its own store in place
//! when it starts, so a Companion that paired over Wi-Fi finds that Pairing in `net.list` over USB.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::asset::refusal;
use crate::net::{CompanionId, PORT};
use crate::pairings::{app_id_hex as hex, parse_app_id};

/// The most Pairings a Device keeps.
const MAX_PAIRINGS: usize = crate::pairings::MAX_PAIRINGS as usize;
const WINDOW: Duration = Duration::from_millis(crate::net::wire::PAIR_WINDOW_MS as u64);

/// What the Device's Pairing store shows: never a key.
#[derive(Debug, Clone, Default)]
pub(crate) struct NetView {
    /// Oldest first.
    pub pairings: Vec<(CompanionId, String)>,
    /// A pairing window is open.
    pub pairing: bool,
    pub pair_failures: u32,
    pub locked_ms: u64,
    /// The Pairing of the network session and its app's name, while there is one.
    pub session: Option<(CompanionId, String)>,
}

/// A Device's Pairing store. The fake net-v2 Device implements it over its own state.
pub(crate) trait NetStore: Send + Sync {
    fn view(&self) -> NetView;
    /// Opens the pairing window, as "Pair new app" does. `Err` with the time left while pairing is locked out.
    fn open_window(&self) -> Result<(), Duration>;
    /// Removes one Pairing and ends its session. Whether there was one.
    fn forget(&self, companion_id: &CompanionId) -> bool;
    /// Removes every Pairing, the failed-code counter and the lockout, closes the window and ends any session.
    fn forget_all(&self);
}

/// The list a bare `MockDeck` keeps.
#[derive(Debug, Default)]
pub(crate) struct LocalNet {
    pub pairings: Vec<(CompanionId, String)>,
    pub window_until: Option<Instant>,
    pub pair_failures: u32,
    pub locked_until: Option<Instant>,
}

impl LocalNet {
    fn locked_left(&self) -> Option<Duration> {
        self.locked_until
            .map(|until| until.saturating_duration_since(Instant::now()))
            .filter(|left| !left.is_zero())
    }

    fn view(&self) -> NetView {
        NetView {
            pairings: self.pairings.clone(),
            pairing: self
                .window_until
                .is_some_and(|until| until > Instant::now()),
            pair_failures: self.pair_failures,
            locked_ms: self.locked_left().map_or(0, |left| left.as_millis() as u64),
            session: None,
        }
    }

    fn open_window(&mut self) -> Result<(), Duration> {
        if let Some(left) = self.locked_left() {
            return Err(left);
        }
        self.window_until = Some(Instant::now() + WINDOW);
        Ok(())
    }

    fn forget(&mut self, companion_id: &CompanionId) -> bool {
        let before = self.pairings.len();
        self.pairings.retain(|(id, _)| id != companion_id);
        self.pairings.len() != before
    }

    fn forget_all(&mut self) {
        *self = LocalNet::default();
    }
}

/// The `net.*` part of the mock Device.
#[derive(Default)]
pub(crate) struct MockNet {
    pub local: LocalNet,
    /// The fake net-v2 Device's store, when one runs in front of this Device.
    pub store: Option<Arc<dyn NetStore>>,
    /// Writes to the Pairing store fail: `net.forget` answers `storage`.
    pub storage_fails: bool,
    /// `net.forget` requests that take effect but get no reply, as a cable pulled at the wrong moment.
    pub lose_forget_replies: u32,
}

impl std::fmt::Debug for MockNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockNet")
            .field("local", &self.local)
            .field("shared", &self.store.is_some())
            .field("storage_fails", &self.storage_fails)
            .finish()
    }
}

/// One Pairing store, whichever holds it.
pub(crate) enum Store<'a> {
    Local(&'a mut LocalNet),
    Shared(Arc<dyn NetStore>),
}

impl Store<'_> {
    fn view(&self) -> NetView {
        match self {
            Store::Local(local) => local.view(),
            Store::Shared(store) => store.view(),
        }
    }

    fn open_window(&mut self) -> Result<(), Duration> {
        match self {
            Store::Local(local) => local.open_window(),
            Store::Shared(store) => store.open_window(),
        }
    }

    fn forget(&mut self, companion_id: &CompanionId) -> bool {
        match self {
            Store::Local(local) => local.forget(companion_id),
            Store::Shared(store) => store.forget(companion_id),
        }
    }

    fn forget_all(&mut self) {
        match self {
            Store::Local(local) => local.forget_all(),
            Store::Shared(store) => store.forget_all(),
        }
    }
}

/// The `net.state` fields.
fn state_fields(view: &NetView) -> Map<String, Value> {
    let fields = json!({
        "listening": true, "port": PORT, "ble": false,
        "session": view.session.as_ref().map(|(id, name)| json!({
            "app_id": hex(id), "app_name": name, "carrier": "wifi", "since_ms": 0,
        })),
        "paired": view.pairings.len(), "pairing": view.pairing,
        "pair_failures": view.pair_failures, "locked_ms": view.locked_ms,
    });
    fields.as_object().cloned().unwrap_or_default()
}

/// Answers one `net.*` request over USB. `storage_fails` makes a forget that removes something answer `storage`, as
/// the Firmware does when the erase doesn't reach flash (the entry is gone from RAM all the same).
pub(crate) fn handle(
    m: &str,
    request: &Map<String, Value>,
    mut store: Store<'_>,
    storage_fails: bool,
) -> Map<String, Value> {
    match m {
        "net.state" => state_fields(&store.view()),
        "net.list" => {
            let view = store.view();
            let mut reply = state_fields(&view);
            reply.insert(
                "free".into(),
                json!(MAX_PAIRINGS.saturating_sub(view.pairings.len())),
            );
            let pairings: Vec<Value> = view
                .pairings
                .iter()
                .map(|(id, name)| json!({"app_id": hex(id), "app_name": name}))
                .collect();
            reply.insert("pairings".into(), Value::Array(pairings));
            reply
        }
        "net.pair" => {
            let view = store.view();
            if !view.pairing && view.pairings.len() >= MAX_PAIRINGS {
                return refusal("full", "Four apps are paired. Forget one first.");
            }
            if !view.pairing && store.open_window().is_err() {
                return refusal(
                    "locked",
                    "Pairing is locked after failed codes; try again later",
                );
            }
            state_fields(&store.view())
        }
        "net.forget" => {
            let all = request.get("all");
            let removed = match (request.get("app_id"), all) {
                (Some(_), Some(_)) => return refusal("args", "Invalid app_id"),
                (Some(app_id), None) => {
                    let Some(id) = app_id.as_str().and_then(parse_app_id) else {
                        return refusal("args", "Invalid app_id");
                    };
                    usize::from(store.forget(&id))
                }
                (None, Some(Value::Bool(true))) => {
                    let count = store.view().pairings.len();
                    store.forget_all();
                    count
                }
                (None, _) => return refusal("args", "Name an app_id, or send all:true"),
            };
            if storage_fails && removed > 0 {
                return refusal("storage", "The pairings could not be erased");
            }
            let mut reply = state_fields(&store.view());
            reply.insert("removed".into(), json!(removed));
            reply
        }
        _ => refusal("args", "Not a Pairing message"),
    }
}
