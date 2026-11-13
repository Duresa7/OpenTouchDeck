use super::{ClientInfo, CommonError, Ctx, EventInfo};
use axum::{
    Extension,
    extract::{
        State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket, rejection::WebSocketUpgradeRejection},
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::{Arc, Weak},
    time::Duration,
};
use ts_rs::TS;

/// Which frames a socket receives. `state` is always on; the others are opt-in per Device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Topic {
    State,
    Console,
    KeyMirror,
    DeviceEvents,
}
/// A frame on the event stream. The type is the frame's `data`.
pub trait Event: Serialize + TS + 'static {
    const NAME: &'static str;
    /// An opt-in topic reaches only sockets that subscribed to it for the frame's Device.
    const TOPIC: Topic = Topic::State;
    /// The Device the frame is about, sent as `device`.
    fn device(&self) -> Option<&str> {
        None
    }
}
/// The first frame. Its fields sit at the top level of the frame, not under `data`.
#[derive(Serialize, Deserialize, TS)]
pub struct Hello {
    pub api: String,
    pub build: String,
    pub session: ClientInfo,
}
impl Event for Hello {
    const NAME: &'static str = "hello";
}
/// The second frame: one key per feature snapshot.
#[derive(Serialize, Deserialize, TS)]
#[ts(rename = "SyncFrame")]
pub struct Sync(pub serde_json::Map<String, serde_json::Value>);
impl Event for Sync {
    const NAME: &'static str = "sync";
}
#[derive(Serialize, Deserialize, TS)]
pub struct AuthRevoked {}
impl Event for AuthRevoked {
    const NAME: &'static str = "auth.revoked";
}
#[derive(Serialize, Deserialize, TS)]
pub struct CompanionStopping {
    pub reason: String,
}
impl Event for CompanionStopping {
    const NAME: &'static str = "companion.stopping";
}
/// Frames of an opt-in topic that the socket's queue had no room for.
#[derive(Serialize, Deserialize, TS)]
pub struct Lagged {
    pub topic: Topic,
    pub device: String,
    pub dropped: u64,
}
impl Event for Lagged {
    const NAME: &'static str = "lagged";
    fn device(&self) -> Option<&str> {
        Some(&self.device)
    }
}
pub(super) fn kernel_events() -> Vec<EventInfo> {
    let feature = super::Feature::new("kernel")
        .event::<Hello>()
        .event::<Sync>()
        .event::<AuthRevoked>()
        .event::<CompanionStopping>()
        .event::<Lagged>();
    feature.events
}
fn frame<E: Event>(event: &E) -> serde_json::Value {
    let mut frame = json!({ "type": E::NAME, "data": event });
    if let Some(device) = event.device() {
        frame["device"] = device.into();
    }
    frame
}

/// JSON framing belongs to this adapter; the Companion only publishes typed changes.
fn encode(change: &crate::companion::Change) -> (serde_json::Value, Topic, Option<String>) {
    use crate::companion::Change;
    fn event<E: Event>(value: &E) -> (serde_json::Value, Topic, Option<String>) {
        (frame(value), E::TOPIC, value.device().map(str::to_owned))
    }
    match change {
        Change::DeviceAdded(value) => event(value),
        Change::DeviceUpdated(value) => event(value),
        Change::DeviceRemoved(value) => event(value),
        Change::NewBoardAdded(value) => event(value),
        Change::NewBoardRemoved(value) => event(value),
        Change::DeviceConsole(value) => event(value),
        Change::DeviceKeyMirror(value) => event(value),
        Change::DeviceEvent(value) => event(value),
        Change::ApprovalRequested(value) => event(value),
        Change::ApprovalResolved(value) => event(value),
        Change::ActionLogged(value) => event(value),
        Change::JobUpdated(value) => event(value),
        Change::IntegrationStatus(value) => event(value),
        Change::Stopping => event(&CompanionStopping {
            reason: "shutdown".into(),
        }),
    }
}

#[derive(Default)]
struct Queue {
    frames: VecDeque<(serde_json::Value, usize)>,
    bytes: usize,
    overflow: bool,
    /// Opt-in frames dropped for lack of room, sent as `lagged` once the queue drains.
    lagged: BTreeMap<(Topic, String), u64>,
}
struct Subscriber {
    queue: Mutex<Queue>,
    topics: Mutex<HashSet<(Topic, String)>>,
    notify: tokio::sync::Notify,
    gets_state_frames: bool,
}
/// What a client may send: `{"op": "subscribe" | "unsubscribe", "topic", "device"}`.
#[derive(Deserialize)]
struct TopicRequest {
    op: String,
    topic: Topic,
    device: crate::devices::DeviceId,
}
/// Fans `state` frames out to every open socket, each with its own bounded queue.
#[derive(Default, Clone)]
pub(crate) struct WebEvents {
    subscribers: Arc<Mutex<Vec<Weak<Subscriber>>>>,
    _observation: Option<Arc<crate::events::Observer>>,
}
impl WebEvents {
    pub(crate) fn new(events: &crate::events::Events) -> Self {
        let web = Self::default();
        let subscribers = Arc::downgrade(&web.subscribers);
        let observation = events.observe(Arc::new(move |change| {
            let Some(subscribers) = subscribers.upgrade() else {
                return;
            };
            let web = Self {
                subscribers,
                _observation: None,
            };
            if matches!(change, crate::companion::Change::Stopping) {
                return;
            }
            let (value, topic, device) = encode(change);
            web.publish_frame(value, topic, device);
        }));
        Self {
            _observation: Some(observation),
            ..web
        }
    }
    pub(crate) fn publish_frame(
        &self,
        frame: serde_json::Value,
        topic: Topic,
        device: Option<String>,
    ) {
        let key = (topic != Topic::State).then(|| (topic, device.unwrap_or_default()));
        let size = frame.to_string().len();
        self.subscribers.lock().retain(|weak| {
            let Some(s) = weak.upgrade() else {
                return false;
            };
            match &key {
                Some(key) if !s.topics.lock().contains(key) => return true,
                None if !s.gets_state_frames => return true,
                _ => {}
            }
            let mut q = s.queue.lock();
            if q.frames.len() >= 1024 || q.bytes + size > 4 * 1024 * 1024 {
                match &key {
                    // An opt-in topic drops the frame and reports it; `state` can't lose one.
                    Some(key) => *q.lagged.entry(key.clone()).or_default() += 1,
                    None => q.overflow = true,
                }
            } else {
                q.bytes += size;
                q.frames.push_back((frame.clone(), size));
            }
            s.notify.notify_one();
            true
        });
    }
    /// Wakes every socket so it rechecks its session.
    pub fn notify(&self) {
        for s in self.subscribers.lock().iter().filter_map(Weak::upgrade) {
            s.notify.notify_one();
        }
    }
    fn subscribe(&self) -> Arc<Subscriber> {
        self.add(true, HashSet::new())
    }
    fn add(&self, gets_state_frames: bool, topics: HashSet<(Topic, String)>) -> Arc<Subscriber> {
        let s = Arc::new(Subscriber {
            queue: Mutex::new(Queue::default()),
            topics: Mutex::new(topics),
            notify: tokio::sync::Notify::new(),
            gets_state_frames,
        });
        self.subscribers.lock().push(Arc::downgrade(&s));
        s
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn listen(&self, topic: Topic, device: &str) -> Listener {
        Listener(self.add(false, HashSet::from([(topic, device.to_owned())])))
    }
}
#[cfg(any(test, feature = "test-hooks"))]
pub(crate) struct Listener(Arc<Subscriber>);
#[cfg(any(test, feature = "test-hooks"))]
impl Listener {
    pub(crate) async fn next(&self, deadline: tokio::time::Instant) -> Option<serde_json::Value> {
        loop {
            let notified = self.0.notify.notified();
            if let Some((mut frame, _)) = {
                let mut queue = self.0.queue.lock();
                let next = queue.frames.pop_front();
                if let Some((_, size)) = &next {
                    queue.bytes -= size;
                }
                next
            } {
                return Some(frame["data"].take());
            }
            tokio::time::timeout_at(deadline, notified).await.ok()?;
        }
    }
}
struct Lease {
    ctx: Ctx,
    id: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let now = self.ctx.clock.now();
        self.ctx.auth.lock().socket_close(&self.id, now);
    }
}
pub(super) async fn upgrade(
    State(ctx): State<Ctx>,
    Extension(client): Extension<ClientInfo>,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, CommonError> {
    #[cfg(any(test, feature = "test-hooks"))]
    ctx.handler_hits
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let ws = ws.map_err(|e| CommonError::invalid("upgrade", e.body_text()))?;
    ctx.auth.lock().socket_open(&client.id, ctx.clock.now())?;
    let lease = Lease {
        ctx: ctx.clone(),
        id: client.id.clone(),
    };
    // Only `touchdeck.v1` is selected, so the bearer entry is never echoed.
    Ok(ws
        .protocols(["touchdeck.v1"])
        .max_message_size(4096)
        .max_frame_size(4096)
        .on_upgrade(move |socket| async move {
            let _lease = lease;
            serve(socket, ctx, client).await;
        }))
}
fn numbered(mut value: serde_json::Value, seq: &mut u64) -> Message {
    value["seq"] = (*seq).into();
    *seq += 1;
    Message::Text(value.to_string().into())
}
/// Every send waits at most this long. A reader that stalls longer loses its socket.
const SEND_TIMEOUT: Duration = Duration::from_secs(2);
type Sink = futures_util::stream::SplitSink<WebSocket, Message>;
async fn send(sink: &mut Sink, message: Message) -> bool {
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, sink.send(message)).await,
        Ok(Ok(()))
    )
}
async fn serve(socket: WebSocket, ctx: Ctx, client: ClientInfo) {
    let subscriber = ctx.web_events.subscribe();
    let (mut sink, mut stream) = socket.split();
    let mut seq = 0u64;
    let mut hello = serde_json::to_value(Hello {
        api: "v1".into(),
        build: env!("TD_BUILD").into(),
        session: client.clone(),
    })
    .unwrap();
    hello["type"] = Hello::NAME.into();
    if !send(&mut sink, numbered(hello, &mut seq)).await {
        return;
    }
    if ctx.auth.lock().active(&client.id, ctx.clock.now()) {
        let state = ctx.snapshot();
        let snapshot = serde_json::to_value(state)
            .expect("snapshot serializes")
            .as_object()
            .unwrap()
            .clone();
        if !send(&mut sink, numbered(frame(&Sync(snapshot)), &mut seq)).await {
            return;
        }
    }
    let mut shutdown = ctx.shutdown.subscribe();
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    ping.tick().await;
    let mut missed = 0;
    // `Some(code)` closes with that code. `None` drops the connection: a send already failed,
    // so a close frame would stall too.
    let close: Option<u16> = loop {
        if !ctx.auth.lock().active(&client.id, ctx.clock.now()) {
            let revoked = numbered(frame(&AuthRevoked {}), &mut seq);
            break send(&mut sink, revoked).await.then_some(4401);
        }
        let next = {
            let mut queue = subscriber.queue.lock();
            if queue.overflow {
                break Some(4008);
            }
            let next = queue.frames.pop_front();
            if let Some((_, size)) = &next {
                queue.bytes -= size;
            }
            match next {
                None => queue.lagged.pop_first().map(|((topic, device), dropped)| {
                    let lagged = Lagged {
                        topic,
                        device,
                        dropped,
                    };
                    (frame(&lagged), 0)
                }),
                next => next,
            }
        };
        if let Some((value, _)) = next {
            if !send(&mut sink, numbered(value, &mut seq)).await {
                break None;
            }
            continue;
        }
        tokio::select! {
            _ = subscriber.notify.notified() => {}
            _ = shutdown.changed() => {
                let stopping = frame(&CompanionStopping { reason: "shutdown".into() });
                break send(&mut sink, numbered(stopping, &mut seq)).await.then_some(1001);
            }
            _ = ping.tick() => {
                if missed >= 2 {
                    break Some(1001);
                }
                missed += 1;
                if !send(&mut sink, Message::Ping(Vec::new().into())).await {
                    break None;
                }
            }
            message = stream.next() => match message {
                Some(Ok(Message::Pong(_))) => missed = 0,
                Some(Ok(Message::Text(text))) => subscribe(&subscriber, &text),
                Some(Ok(Message::Close(_))) => break Some(1000),
                // A protocol error, such as a frame over the size limit, or a closed stream.
                None | Some(Err(_)) => break None,
                _ => {}
            },
        }
    };
    if let Some(code) = close {
        let frame = CloseFrame {
            code,
            reason: "".into(),
        };
        send(&mut sink, Message::Close(Some(frame))).await;
    }
}

/// Applies a `subscribe` or `unsubscribe` request. Anything else is ignored and the socket
/// stays open. `state` is always on, so a request for it changes nothing.
fn subscribe(subscriber: &Subscriber, text: &str) {
    let Ok(request) = serde_json::from_str::<TopicRequest>(text) else {
        return;
    };
    if request.topic == Topic::State {
        return;
    }
    let key = (request.topic, request.device.as_str().to_owned());
    let mut topics = subscriber.topics.lock();
    match request.op.as_str() {
        "subscribe" => {
            topics.insert(key);
        }
        "unsubscribe" => {
            topics.remove(&key);
        }
        _ => {}
    }
}

#[cfg(test)]
mod delivery_review_tests {
    use super::*;
    #[test]
    fn review_delivery_error_reaches_event_stream() {
        let events = crate::events::Events::default();
        let web = WebEvents::new(&events);
        let subscriber = web.subscribe();
        let service = crate::integrations::Service::new(true, events);
        let message = "The Device is out of space for cover art. Remove unused Assets, then retry.";
        service.delivered(
            "this_pc",
            td_host::integrations::Delivery {
                device: "mock0001".into(),
                sent: 0,
                error: Some(message.into()),
            },
        );
        assert!(
            subscriber
                .queue
                .lock()
                .frames
                .iter()
                .any(|(frame, _)| frame.to_string().contains(message))
        );
    }
}
