use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::io;
// The tests' byte-stream fakes still speak Read and Write under the USB carrier.
#[cfg(test)]
use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;
use td_core::{
    Body, Frame, FrameError, Hello, Item, Pong, Reply, ReplyError, Request, Role,
    frame::PAYLOAD_MAX,
};

use crate::Transport;
use crate::carrier::{
    Carrier, FrameReader, FrameWriter, Outbound, Received, Released, SendWindow, UsbCarrier,
};
use crate::xfer::LayoutState;

const CONSOLE_LINE_MAX: usize = 16 * 1024;
const INCOMING_CAPACITY: usize = 256;
const CONSOLE_QUEUE_LIMIT: usize = 192;
const TRANSFER_CAPACITY: usize = 64;

#[derive(Default)]
struct ConsoleLine {
    bytes: Vec<u8>,
    discarding: bool,
}

const WRITE_DEADLINE: Duration = Duration::from_secs(2);
const LAYOUT_REPLY_GRACE: Duration = Duration::from_secs(3);
/// Why a session closes when the deck no longer counts the app as online.
pub const DROPPED: &str = "the Device dropped the session";

#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    pub ping_every: Option<Duration>,
    pub ping_timeout: Duration,
    /// Closes the session when a ping reply's uptime goes backwards, so the caller can say
    /// `hello` again: a restarted deck counts the app as offline until it does.
    pub close_on_restart: bool,
    /// Closes the session with [`DROPPED`] when the deck says it no longer counts the app as
    /// online: a heartbeat `link.state` reads `online: false`, or a `v2.` request is refused
    /// `not-negotiated` after `hello` selected protocol 2. The deck drops the session after
    /// 5 seconds without a frame and refuses protocol 2 until the next `hello`.
    pub close_on_drop: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            ping_every: Some(Duration::from_secs(2)),
            ping_timeout: Duration::from_secs(2),
            close_on_restart: false,
            close_on_drop: false,
        }
    }
}

/// A chunk or an ack of a transfer, with the frame `seq` an ack must echo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XferFrame {
    Chunk {
        seq: u8,
        xfer: u8,
        idx: u16,
        data: Vec<u8>,
    },
    Ack {
        xfer: u8,
        next: u16,
    },
}

/// What the deck sends that is not a reply: events, console text lines, and the end.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Event(Value),
    Line(String),
    Closed(String),
    Overflow,
}

/// Bounded FIFO traffic. Excess console lines are dropped. Event loss emits one Overflow
/// after retained events; events resume after the consumer receives that marker.
pub struct IncomingReceiver {
    receiver: Receiver<Incoming>,
    queued: Arc<AtomicUsize>,
    overflow: Arc<AtomicBool>,
}

impl IncomingReceiver {
    fn consumed(&self, item: &Incoming) {
        self.queued.fetch_sub(1, Ordering::AcqRel);
        if matches!(item, Incoming::Overflow) {
            self.overflow.store(false, Ordering::Release);
        }
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Incoming, RecvTimeoutError> {
        self.receiver
            .recv_timeout(timeout)
            .inspect(|item| self.consumed(item))
    }

    pub fn try_recv(&self) -> Result<Incoming, mpsc::TryRecvError> {
        self.receiver.try_recv().inspect(|item| self.consumed(item))
    }

    pub fn try_iter(&self) -> impl Iterator<Item = Incoming> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }
}

impl Iterator for IncomingReceiver {
    type Item = Incoming;
    fn next(&mut self) -> Option<Incoming> {
        self.receiver
            .recv()
            .ok()
            .inspect(|item| self.consumed(item))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Timeout(Duration),
    Closed(String),
    /// The request went out and the session ended before its reply came. The outcome is
    /// unknown: the Device may not have got it, or may have acted and lost the reply.
    Unanswered(String),
    Busy,
    TransferBusy,
    ConsoleBusy,
    Io(String),
    Reply(ReplyError),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::Timeout(after) => write!(f, "no reply within {} ms", after.as_millis()),
            LinkError::Closed(reason) => write!(f, "link closed: {reason}"),
            LinkError::Unanswered(reason) => write!(
                f,
                "the link closed before the Device answered, so whether it did this is unknown: {reason}"
            ),
            LinkError::Busy => f.write_str("255 requests are already waiting"),
            LinkError::TransferBusy => f.write_str("a transfer is already running"),
            LinkError::ConsoleBusy => f.write_str("a layout state reply is still pending"),
            LinkError::Io(error) => write!(f, "write failed: {error}"),
            LinkError::Reply(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LinkError {}

struct Pending {
    last: u8,
    waiting: HashMap<u8, SyncSender<Reply>>,
}

struct LayoutReply {
    waiting: SyncSender<Value>,
    expires: Instant,
}

struct Shared {
    writer: Mutex<Box<dyn FrameWriter>>,
    pending: Mutex<Pending>,
    open: AtomicBool,
    reason: Mutex<String>,
    incoming: SyncSender<Incoming>,
    queued: Arc<AtomicUsize>,
    overflow: Arc<AtomicBool>,
    xfer: Mutex<Option<SyncSender<XferFrame>>>,
    xfer_ready: Condvar,
    transferring: AtomicBool,
    layout_reply: Mutex<Option<LayoutReply>>,
    close_on_drop: bool,
    protocol_2: AtomicBool,
}

impl Shared {
    fn shutdown(&self, reason: &str) {
        if self.open.swap(false, Ordering::AcqRel) {
            *self.reason.lock().unwrap_or_else(PoisonError::into_inner) = reason.to_owned();
            self.xfer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            self.xfer_ready.notify_all();
            self.pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .waiting
                .clear();
            self.layout_reply
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            self.queued.fetch_add(1, Ordering::AcqRel);
            if self
                .incoming
                .try_send(Incoming::Closed(reason.to_owned()))
                .is_err()
            {
                self.queued.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    fn publish(&self, item: Incoming) {
        if !self.open.load(Ordering::Acquire) {
            return;
        }
        let is_event = matches!(item, Incoming::Event(_));
        if is_event && self.overflow.load(Ordering::Acquire) {
            return;
        }
        let limit = if is_event {
            INCOMING_CAPACITY
        } else {
            CONSOLE_QUEUE_LIMIT
        };
        if self.queued.fetch_add(1, Ordering::AcqRel) >= limit
            || self.incoming.try_send(item).is_err()
        {
            self.queued.fetch_sub(1, Ordering::AcqRel);
            if is_event && !self.overflow.swap(true, Ordering::AcqRel) {
                self.queued.fetch_add(1, Ordering::AcqRel);
                if self.incoming.try_send(Incoming::Overflow).is_err() {
                    self.queued.fetch_sub(1, Ordering::AcqRel);
                    self.overflow.store(false, Ordering::Release);
                }
            }
        }
    }

    fn closed(&self) -> LinkError {
        LinkError::Closed(self.reason.lock().unwrap().clone())
    }

    fn send(&self, out: Outbound<'_>) -> io::Result<()> {
        let mut writer = self.writer.lock().unwrap();
        self.send_locked(&mut **writer, out, Instant::now() + WRITE_DEADLINE)
    }

    fn send_locked(
        &self,
        writer: &mut dyn FrameWriter,
        out: Outbound<'_>,
        deadline: Instant,
    ) -> io::Result<()> {
        writer.send(out, &SendWindow::new(deadline, &self.open))
    }

    fn next_seq(pending: &mut Pending) -> Result<u8, LinkError> {
        let seq = (1..=255u16)
            .map(|step| ((u16::from(pending.last) + step - 1) % 255 + 1) as u8)
            .find(|seq| !pending.waiting.contains_key(seq))
            .ok_or(LinkError::Busy)?;
        pending.last = seq;
        Ok(seq)
    }

    fn request(&self, request: &Request, timeout: Duration) -> Result<Reply, LinkError> {
        self.call(request.name(), request.to_payload(), timeout)
    }

    fn call(&self, name: &str, payload: Vec<u8>, timeout: Duration) -> Result<Reply, LinkError> {
        let deadline = Instant::now() + timeout;
        let (slot, reply) = mpsc::sync_channel(1);
        let seq = {
            let mut pending = self.pending.lock().unwrap();
            if !self.open.load(Ordering::Acquire) {
                return Err(self.closed());
            }
            let seq = Shared::next_seq(&mut pending)?;
            pending.waiting.insert(seq, slot);
            seq
        };
        let forget = || self.pending.lock().unwrap().waiting.remove(&seq);
        let frame = Frame::message(Role::Request(seq), payload);
        fits(&frame)
            .map_err(|error| LinkError::Io(format!("request does not fit one frame: {error}")))?;
        let sent = (|| {
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(LinkError::Timeout(timeout));
                }
                if !self.open.load(Ordering::Acquire) {
                    return Err(self.closed());
                }
                match self.writer.try_lock() {
                    Ok(mut writer) => {
                        if Instant::now() >= deadline {
                            return Err(LinkError::Timeout(timeout));
                        }
                        if !self.transferring.load(Ordering::Acquire) {
                            writer
                                .start_operation()
                                .map_err(|error| LinkError::Io(error.to_string()))?;
                        }
                        let result = self.send_locked(
                            &mut **writer,
                            Outbound::Frame(&frame),
                            deadline.min(Instant::now() + WRITE_DEADLINE),
                        );
                        return if Instant::now() >= deadline {
                            Err(LinkError::Timeout(timeout))
                        } else {
                            result.map_err(|error| LinkError::Io(error.to_string()))
                        };
                    }
                    Err(TryLockError::WouldBlock) => {
                        thread::sleep(left.min(Duration::from_millis(1)));
                    }
                    Err(TryLockError::Poisoned(error)) => panic!("{error}"),
                }
            }
        })();
        if let Err(error) = sent {
            forget();
            return Err(error);
        }
        match reply.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(reply) if reply.m == name => {
                self.watch(&reply);
                Ok(reply)
            }
            Ok(reply) => Err(LinkError::Reply(ReplyError::Fields(format!(
                "reply names {:?}, not {name:?}",
                reply.m
            )))),
            Err(RecvTimeoutError::Timeout) => {
                forget();
                Err(LinkError::Timeout(timeout))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(LinkError::Unanswered(self.reason.lock().unwrap().clone()))
            }
        }
    }

    fn watch(&self, reply: &Reply) {
        if reply.m == "hello" {
            let selected = reply.ok && reply.fields.get("proto").and_then(Value::as_u64) == Some(2);
            self.protocol_2.store(selected, Ordering::Release);
        } else if self.close_on_drop
            && reply.m.starts_with("v2.")
            && !reply.ok
            && reply.fields.get("reason").and_then(Value::as_str) == Some("not-negotiated")
            && self.protocol_2.load(Ordering::Acquire)
        {
            self.shutdown(DROPPED);
        }
    }

    fn route(&self, item: Item, buffer: &mut ConsoleLine) {
        match item {
            Item::Text(bytes) => {
                for byte in bytes {
                    if buffer.discarding {
                        if byte == b'\n' {
                            buffer.discarding = false;
                            self.publish(Incoming::Line("[console line truncated]".into()));
                        }
                        continue;
                    }
                    let line = &mut buffer.bytes;
                    if byte == b'\n' {
                        let text =
                            String::from_utf8_lossy(line.strip_suffix(b"\r").unwrap_or(line))
                                .into_owned();
                        line.clear();
                        if let Some(at) = text.find("@td ")
                            && let Ok(reply) = serde_json::from_str::<Value>(&text[at + 4..])
                            && ((reply["cmd"] == "layout"
                                && (reply["op"] == "state" || reply["ok"] == false))
                                || (reply["cmd"] == "console"
                                    && reply["ok"] == false
                                    && reply["error"] == "input overflow"))
                            && let Some(waiting) = self.layout_reply.lock().unwrap().take()
                        {
                            let _ = waiting.waiting.send(reply);
                        }
                        self.publish(Incoming::Line(text));
                    } else {
                        if line.len() == CONSOLE_LINE_MAX {
                            line.clear();
                            buffer.discarding = true;
                        } else {
                            line.push(byte);
                        }
                    }
                }
            }
            Item::Frame(Frame {
                role: Role::Response(seq),
                body: Body::Message(payload),
            }) => {
                if let Ok(reply) = Reply::parse(&payload)
                    && let Some(slot) = self.pending.lock().unwrap().waiting.remove(&seq)
                {
                    let _ = slot.send(reply);
                }
            }
            Item::Frame(Frame {
                role: Role::Event,
                body: Body::Message(payload),
            }) => {
                if let Ok(event @ Value::Object(_)) = serde_json::from_slice::<Value>(&payload) {
                    self.publish(Incoming::Event(event));
                }
            }
            Item::Frame(Frame {
                role,
                body: Body::Chunk { xfer, idx, data },
            }) => self.to_xfer(XferFrame::Chunk {
                seq: role.seq(),
                xfer,
                idx,
                data,
            }),
            Item::Frame(Frame {
                body: Body::Ack { xfer, next },
                ..
            }) => self.to_xfer(XferFrame::Ack { xfer, next }),
            Item::Frame(_) => {}
        }
    }

    fn to_xfer(&self, frame: XferFrame) {
        if let Some(sink) = self.xfer.lock().unwrap().as_ref() {
            let _ = sink.try_send(frame);
        }
    }
}

/// The one transfer a session runs at a time. Chunks and acks reach [`Transfer::frames`]
/// until it drops. `begin` and `finish` wrap the protocol-specific requests; a guard
/// dropped between them closes the Carrier to abort the Device's transfer. A refused
/// begin or an already-present Asset needs no cancellation.
pub struct Transfer<'a> {
    session: &'a Session,
    pub frames: Receiver<XferFrame>,
    unfinished: bool,
    deadline: Cell<Option<(Instant, Duration)>>,
}

impl Transfer<'_> {
    /// Runs the begin request under this operation's admission. If its outcome is
    /// unknown or the caller abandons it, closing the session aborts Device state.
    pub fn begin<E>(&mut self, request: impl FnOnce() -> Result<Reply, E>) -> Result<Reply, E> {
        self.unfinished = true;
        let reply = request()?;
        self.unfinished =
            reply.ok && reply.fields.get("present").and_then(Value::as_bool) != Some(true);
        Ok(reply)
    }

    /// Runs completion before releasing admission. Failed completion has an unknown
    /// outcome, so Drop closes the session rather than allowing another operation.
    pub fn finish<T, E>(&mut self, request: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let result = request();
        if result.is_ok() {
            self.unfinished = false;
        }
        result
    }

    /// Bounds data movement by size, in addition to each protocol's idle/retry limits.
    pub fn data_deadline(&self, bytes: usize) -> Instant {
        let timeout = Duration::from_secs(5 + (bytes as u64).div_ceil(4096));
        let deadline = Instant::now() + timeout;
        self.deadline.set(Some((deadline, timeout)));
        deadline
    }

    /// Writes one chunk as a request frame with a fresh `seq`, which the receiver's ack echoes.
    pub fn send_chunk(&self, xfer: u8, idx: u16, data: &[u8]) -> Result<(), LinkError> {
        let shared = &self.session.shared;
        let seq = Shared::next_seq(&mut shared.pending.lock().unwrap())?;
        self.write(Frame {
            role: Role::Request(seq),
            body: Body::Chunk {
                xfer,
                idx,
                data: data.to_vec(),
            },
        })
    }

    /// Acks a received chunk with the next index wanted, echoing the chunk's `seq`.
    pub fn send_ack(&self, seq: u8, xfer: u8, next: u16) -> Result<(), LinkError> {
        self.write(Frame {
            role: Role::Response(seq),
            body: Body::Ack { xfer, next },
        })
    }

    fn write(&self, frame: Frame) -> Result<(), LinkError> {
        let shared = &self.session.shared;
        if let Some((deadline, timeout)) = self.deadline.get()
            && Instant::now() >= deadline
        {
            return Err(LinkError::Timeout(timeout));
        }
        if !shared.open.load(Ordering::Acquire) {
            return Err(shared.closed());
        }
        fits(&frame).map_err(|error| LinkError::Io(format!("frame does not encode: {error}")))?;
        shared
            .send(Outbound::Frame(&frame))
            .map_err(|error| LinkError::Io(error.to_string()))
    }
}

impl Drop for Transfer<'_> {
    fn drop(&mut self) {
        if self.unfinished {
            self.session
                .shared
                .shutdown("transfer abandoned; reconnect to go on");
        }
        let mut slot = self
            .session
            .shared
            .xfer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.take();
        self.session
            .shared
            .transferring
            .store(false, Ordering::Release);
        self.session.shared.xfer_ready.notify_all();
    }
}

/// One link session over one [`Carrier`]. Replies match requests by `seq`, everything
/// else arrives on the [`Incoming`] channel, and a `ping` every 2 seconds proves the deck
/// is still there. Closing or dropping the session joins its threads, which releases
/// the carrier.
pub struct Session {
    shared: Arc<Shared>,
    console: Mutex<()>,
    stop: Mutex<Option<Sender<()>>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Session {
    /// Opens a session over USB: a [`UsbCarrier`] on the transport's byte stream.
    pub fn open(
        transport: Box<dyn Transport>,
        config: SessionConfig,
    ) -> io::Result<(Session, IncomingReceiver)> {
        Session::over(Box::new(UsbCarrier::new(transport)), config)
    }

    /// Opens a session over any carrier.
    pub fn over(
        carrier: Box<dyn Carrier>,
        config: SessionConfig,
    ) -> io::Result<(Session, IncomingReceiver)> {
        let (reader, writer) = carrier.split()?;
        let (incoming, events) = mpsc::sync_channel(INCOMING_CAPACITY + 2);
        let queued = Arc::new(AtomicUsize::new(0));
        let overflow = Arc::new(AtomicBool::new(false));
        let events = IncomingReceiver {
            receiver: events,
            queued: queued.clone(),
            overflow: overflow.clone(),
        };
        let shared = Arc::new(Shared {
            writer: Mutex::new(writer),
            pending: Mutex::new(Pending {
                last: 0,
                waiting: HashMap::new(),
            }),
            open: AtomicBool::new(true),
            reason: Mutex::new(String::new()),
            incoming,
            queued,
            overflow,
            xfer: Mutex::new(None),
            xfer_ready: Condvar::new(),
            transferring: AtomicBool::new(false),
            layout_reply: Mutex::new(None),
            close_on_drop: config.close_on_drop,
            protocol_2: AtomicBool::new(false),
        });
        let (stop, stopped) = mpsc::channel::<()>();
        let mut threads = vec![thread::spawn({
            let shared = shared.clone();
            move || read_loop(&shared, reader)
        })];
        if let Some(every) = config.ping_every {
            let shared = shared.clone();
            threads.push(thread::spawn(move || {
                ping_loop(&shared, &stopped, every, config)
            }));
        }
        Ok((
            Session {
                shared,
                console: Mutex::new(()),
                stop: Mutex::new(Some(stop)),
                threads: Mutex::new(threads),
            },
            events,
        ))
    }

    pub fn request(&self, request: &Request, timeout: Duration) -> Result<Reply, LinkError> {
        self.shared.request(request, timeout)
    }

    /// Sends a message request that `td_core::Request` does not name, as JSON with its `m`.
    pub fn call(&self, payload: &Value, timeout: Duration) -> Result<Reply, LinkError> {
        let name = payload
            .get("m")
            .and_then(Value::as_str)
            .ok_or_else(|| LinkError::Io("a message needs a string m".into()))?;
        let bytes = serde_json::to_vec(payload).expect("JSON values serialize");
        self.shared.call(name, bytes, timeout)
    }

    /// Sends a message request that is already JSON, so its fields keep their order on the wire.
    pub fn call_payload(
        &self,
        name: &str,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> Result<Reply, LinkError> {
        self.shared.call(name, payload, timeout)
    }

    /// Waits up to ten seconds for the session's transfer, waking on release or close.
    /// The guard owns admission through begin, chunks and completion on every Carrier.
    pub fn transfer(&self) -> Result<Transfer<'_>, LinkError> {
        let wait = Duration::from_secs(10);
        let deadline = Instant::now() + wait;
        let mut slot = self.shared.xfer.lock().unwrap();
        while slot.is_some() && self.is_open() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(LinkError::Timeout(wait));
            }
            slot = self.shared.xfer_ready.wait_timeout(slot, left).unwrap().0;
        }
        if !self.is_open() {
            return Err(self.shared.closed());
        }
        loop {
            if !self.is_open() {
                return Err(self.shared.closed());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(LinkError::Timeout(wait));
            }
            match self.shared.writer.try_lock() {
                Ok(mut writer) => {
                    writer
                        .start_operation()
                        .map_err(|error| LinkError::Io(error.to_string()))?;
                    self.shared.transferring.store(true, Ordering::Release);
                    break;
                }
                Err(TryLockError::WouldBlock) => thread::sleep(left.min(Duration::from_millis(1))),
                Err(TryLockError::Poisoned(error)) => panic!("{error}"),
            }
        }
        let (sink, frames) = mpsc::sync_channel(TRANSFER_CAPACITY);
        *slot = Some(sink);
        Ok(Transfer {
            session: self,
            frames,
            unfinished: false,
            deadline: Cell::new(None),
        })
    }

    pub fn hello(&self, timeout: Duration) -> Result<Hello, LinkError> {
        self.request(&Request::Hello, timeout)?
            .fields()
            .map_err(LinkError::Reply)
    }

    /// Writes one console line. The deck answers it as text on the [`Incoming`] channel.
    pub fn send_line(&self, line: &str) -> Result<(), LinkError> {
        let _console = self.console.lock().unwrap();
        self.write_line(line)
    }

    /// Reads the displayed layout through the firmware's existing `layout state` console
    /// command. Its reply still reaches [`Incoming`], including when an event handler asks.
    pub fn layout_state(&self, timeout: Duration) -> Result<LayoutState, LinkError> {
        let _console = self.console.lock().unwrap();
        let (waiting, reply) = mpsc::sync_channel(1);
        {
            let mut slot = self.shared.layout_reply.lock().unwrap();
            if slot
                .as_ref()
                .is_some_and(|reply| reply.expires > Instant::now())
            {
                return Err(LinkError::ConsoleBusy);
            }
            *slot = Some(LayoutReply {
                waiting,
                expires: Instant::now() + timeout + LAYOUT_REPLY_GRACE,
            });
        }
        if let Err(error) = self.write_line("layout state") {
            self.shared.layout_reply.lock().unwrap().take();
            return Err(error);
        }
        let value = match reply.recv_timeout(timeout) {
            Ok(value) => value,
            // Console replies have no request ID. Allow late replies to drain for a bounded
            // grace period, then permit a retry even when the original reply was lost.
            Err(RecvTimeoutError::Timeout) => {
                if let Some(reply) = self.shared.layout_reply.lock().unwrap().as_mut() {
                    reply.expires = Instant::now() + LAYOUT_REPLY_GRACE;
                }
                return Err(LinkError::Timeout(timeout));
            }
            Err(RecvTimeoutError::Disconnected) => return Err(self.shared.closed()),
        };
        if value["ok"] != true {
            return Err(LinkError::Reply(ReplyError::Fields(value.to_string())));
        }
        serde_json::from_value(value)
            .map_err(|error| LinkError::Reply(ReplyError::Fields(error.to_string())))
    }

    fn write_line(&self, line: &str) -> Result<(), LinkError> {
        if line.contains(['\r', '\n']) {
            return Err(LinkError::Io("a console line holds no CR or LF".into()));
        }
        if !self.is_open() {
            return Err(self.shared.closed());
        }
        self.shared
            .send(Outbound::Line(line))
            .map_err(|error| LinkError::Io(error.to_string()))
    }

    pub fn is_open(&self) -> bool {
        self.shared.open.load(Ordering::Acquire)
    }

    pub fn close(&self) {
        self.stop
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.shared.shutdown("closed by the app");
        for thread in self
            .threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            let _ = thread.join();
        }
        *self
            .shared
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(Released);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

/// Checks the link rule every carrier shares: a frame's payload fits [`PAYLOAD_MAX`].
fn fits(frame: &Frame) -> Result<(), FrameError> {
    let len = match &frame.body {
        Body::Message(payload) => payload.len(),
        Body::Chunk { data, .. } => data.len() + 3,
        Body::Ack { .. } => 3,
    };
    if len > PAYLOAD_MAX {
        return Err(FrameError::Length);
    }
    Ok(())
}

fn read_loop(shared: &Shared, mut reader: Box<dyn FrameReader>) {
    let mut line = ConsoleLine::default();
    while shared.open.load(Ordering::Acquire) {
        match reader.recv() {
            Ok(Received::Ended) => {
                shared.shutdown("the deck closed the stream");
                break;
            }
            Ok(Received::Items(items)) => {
                for item in items {
                    shared.route(item, &mut line);
                }
            }
            Err(error) => {
                shared.shutdown(&format!("read failed: {error}"));
                break;
            }
        }
    }
    *shared.writer.lock().unwrap_or_else(PoisonError::into_inner) = Box::new(Released);
}

fn ping_loop(shared: &Shared, stop: &Receiver<()>, every: Duration, config: SessionConfig) {
    let mut uptime = 0;
    for n in 1u64.. {
        if !matches!(stop.recv_timeout(every), Err(RecvTimeoutError::Timeout)) {
            return;
        }
        match shared.request(&Request::Ping { n: Some(n) }, config.ping_timeout) {
            Ok(reply) => {
                let Ok(Pong { t, .. }) = reply.fields() else {
                    continue;
                };
                if config.close_on_restart && t < uptime {
                    return shared.shutdown("the deck restarted");
                }
                uptime = t;
            }
            Err(LinkError::Timeout(_)) => {
                return shared.shutdown("the deck stopped answering ping");
            }
            Err(error) => return shared.shutdown(&format!("heartbeat failed: {error}")),
        }
        if !config.close_on_drop {
            continue;
        }
        match shared.request(&Request::LinkState, config.ping_timeout) {
            Ok(reply) if reply.ok && reply.fields.get("online") == Some(&Value::Bool(false)) => {
                return shared.shutdown(DROPPED);
            }
            Ok(_) => {}
            Err(LinkError::Timeout(_)) => {
                return shared.shutdown("the deck stopped answering ping");
            }
            Err(error) => return shared.shutdown(&format!("heartbeat failed: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockDeck;
    use td_core::{Event, IconPage};

    fn quiet() -> SessionConfig {
        SessionConfig {
            ping_every: None,
            ping_timeout: Duration::from_millis(200),
            close_on_restart: false,
            close_on_drop: false,
        }
    }

    fn until<T>(events: &IncomingReceiver, mut pick: impl FnMut(Incoming) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if let Ok(item) = events.recv_timeout(left)
                && let Some(found) = pick(item)
            {
                return found;
            }
        }
        panic!("nothing matched within 5 s");
    }

    fn route_event(session: &Session, event: Value) {
        session.shared.route(
            Item::Frame(Frame::message(
                Role::Event,
                serde_json::to_vec(&event).unwrap(),
            )),
            &mut ConsoleLine::default(),
        );
    }

    #[test]
    fn closing_wakes_transfer_admission_and_the_active_receiver() {
        let deck = MockDeck::new();
        let (session, _) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        let active = session.transfer().unwrap();
        thread::scope(|scope| {
            let (started, waiting) = mpsc::channel();
            let session = &session;
            let next = scope.spawn(move || {
                started.send(()).unwrap();
                matches!(session.transfer(), Err(LinkError::Closed(_)))
            });
            waiting.recv().unwrap();
            session.close();
            assert!(next.join().unwrap());
            assert_eq!(
                active.frames.recv_timeout(Duration::from_millis(100)),
                Err(RecvTimeoutError::Disconnected)
            );
        });
    }

    #[test]
    fn console_backlog_preserves_key_release() {
        let (session, events) = Session::open(Box::new(BrokenTransport), quiet()).unwrap();
        let down = serde_json::json!({"m":"key.report","seq":1,"type":"kbd","mods":1,"keys":[4]});
        let up = serde_json::json!({"m":"key.report","seq":2,"type":"kbd","mods":0,"keys":[]});
        route_event(&session, down.clone());
        for n in 0..255 {
            session.shared.route(
                Item::Text(format!("log {n}\n").into_bytes()),
                &mut ConsoleLine::default(),
            );
        }
        route_event(&session, up.clone());
        let received: Vec<_> = events.try_iter().collect();
        assert_eq!(received.first(), Some(&Incoming::Event(down)));
        assert_eq!(received.last(), Some(&Incoming::Event(up)));
        assert!(session.is_open());
        let lines: Vec<_> = received
            .into_iter()
            .filter_map(|item| match item {
                Incoming::Line(line) => Some(line),
                _ => None,
            })
            .collect();
        assert!(lines.len() <= 192);
        for (n, line) in lines.iter().enumerate() {
            assert_eq!(line, &format!("log {n}"));
        }
    }

    #[test]
    fn event_flood_resyncs_after_the_events_that_fit() {
        let (session, events) = Session::open(Box::new(BrokenTransport), quiet()).unwrap();
        for seq in 0..10000 {
            route_event(&session, serde_json::json!({"m":"press","seq":seq}));
        }
        let received: Vec<_> = events.try_iter().collect();
        assert_eq!(received.len(), 257);
        for (seq, item) in received[..256].iter().enumerate() {
            assert_eq!(
                item,
                &Incoming::Event(serde_json::json!({"m":"press","seq":seq}))
            );
        }
        assert_eq!(received.last(), Some(&Incoming::Overflow));
        assert!(session.is_open());
        route_event(&session, serde_json::json!({"m":"press","seq":10001}));
        assert_eq!(
            events.try_recv().unwrap(),
            Incoming::Event(serde_json::json!({"m":"press","seq":10001}))
        );
    }

    #[test]
    fn overflow_rearms_on_each_receive_api_and_keeps_the_close_slot() {
        for receive in 0..3 {
            let (session, mut events) = Session::open(Box::new(BrokenTransport), quiet()).unwrap();
            for _ in 0..2 {
                for seq in 0..10000 {
                    route_event(&session, serde_json::json!({"m":"press","seq":seq}));
                }
                for _ in 0..256 {
                    assert!(matches!(events.try_recv(), Ok(Incoming::Event(_))));
                }
                let marker = match receive {
                    0 => events.try_recv().unwrap(),
                    1 => events.recv_timeout(Duration::from_secs(1)).unwrap(),
                    _ => events.next().unwrap(),
                };
                assert_eq!(marker, Incoming::Overflow);
                assert!(events.try_recv().is_err());
            }
            for seq in 0..10000 {
                route_event(&session, serde_json::json!({"m":"press","seq":seq}));
            }
            session.close();
            let received: Vec<_> = events.try_iter().collect();
            assert_eq!(received.len(), 258);
            assert_eq!(received[256], Incoming::Overflow);
            assert!(matches!(received[257], Incoming::Closed(_)));
        }
    }

    #[test]
    fn replies_match_their_requests_by_sequence() {
        let deck = MockDeck::new();
        let (session, _events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        let session = Arc::new(session);
        let workers: Vec<_> = (0..4u64)
            .map(|worker| {
                let session = session.clone();
                thread::spawn(move || {
                    for i in 0..100u64 {
                        let n = worker * 1000 + i;
                        let pong: Pong = session
                            .request(&Request::Ping { n: Some(n) }, Duration::from_secs(2))
                            .unwrap()
                            .fields()
                            .unwrap();
                        assert_eq!(pong.n, Some(n));
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(
            deck.received().iter().filter(|m| m["m"] == "ping").count(),
            400
        );
    }

    #[test]
    fn an_event_during_a_request_reaches_the_channel_and_the_reply_still_matches() {
        let deck = MockDeck::new();
        deck.delay_replies(Duration::from_millis(150));
        let (session, events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        let emitter = {
            let deck = deck.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(40));
                deck.emit(&Event::Press {
                    page: "apps".into(),
                    tile: "launch".into(),
                    kind: "launch".into(),
                    part: None,
                    seq: 1,
                    t: 10,
                });
            })
        };
        let hello = session.hello(Duration::from_secs(2)).unwrap();
        emitter.join().unwrap();
        assert!(hello.is_deck());
        let event = until(&events, |item| match item {
            Incoming::Event(event) => Some(event),
            _ => None,
        });
        assert_eq!(
            serde_json::from_value::<Event>(event).unwrap(),
            Event::Press {
                page: "apps".into(),
                tile: "launch".into(),
                kind: "launch".into(),
                part: None,
                seq: 1,
                t: 10
            }
        );
    }

    #[test]
    fn a_silent_deck_times_out_and_a_late_reply_is_dropped() {
        let deck = MockDeck::new();
        let (session, _events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        deck.stop();
        let started = Instant::now();
        assert_eq!(
            session.request(&Request::Hello, Duration::from_millis(300)),
            Err(LinkError::Timeout(Duration::from_millis(300)))
        );
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(session.is_open());
    }

    struct StalledWriter {
        entered: Sender<()>,
        release: Receiver<()>,
        writes: Arc<AtomicUsize>,
    }

    impl Write for StalledWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Transport for StalledWriter {
        fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
            Ok((Box::new(TimeoutReader), self))
        }
    }

    #[test]
    fn request_timeout_includes_stalled_write_lock_without_a_late_write() {
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let writes = Arc::new(AtomicUsize::new(0));
        let (session, _events) = Session::open(
            Box::new(StalledWriter {
                entered,
                release: blocked,
                writes: writes.clone(),
            }),
            quiet(),
        )
        .unwrap();
        let session = Arc::new(session);
        let first = thread::spawn({
            let session = session.clone();
            move || session.send_line("version")
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let (done, result) = mpsc::channel();
        let timeout = Duration::from_millis(100);
        let second = thread::spawn({
            let session = session.clone();
            move || {
                done.send(session.request(&Request::Hello, timeout))
                    .unwrap()
            }
        });
        let timely = result.recv_timeout(Duration::from_millis(600));
        release.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap();
        assert_eq!(
            writes.load(Ordering::SeqCst),
            1,
            "expired request wrote after release"
        );
        assert_eq!(timely, Ok(Err(LinkError::Timeout(timeout))));
        assert!(session.is_open());
    }

    #[test]
    fn heartbeat_timeout_includes_stalled_write_lock() {
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let writes = Arc::new(AtomicUsize::new(0));
        let (session, events) = Session::open(
            Box::new(StalledWriter {
                entered,
                release: blocked,
                writes: writes.clone(),
            }),
            quiet(),
        )
        .unwrap();
        let session = Arc::new(session);
        let first = thread::spawn({
            let session = session.clone();
            move || session.send_line("version")
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let (stop, stopped) = mpsc::channel();
        let heartbeat = thread::spawn({
            let shared = session.shared.clone();
            move || ping_loop(&shared, &stopped, Duration::from_millis(10), quiet())
        });
        let closed = events.recv_timeout(Duration::from_millis(800));
        let was_open = session.is_open();
        stop.send(()).ok();
        release.send(()).unwrap();
        first.join().unwrap().ok();
        heartbeat.join().unwrap();
        assert_eq!(
            closed,
            Ok(Incoming::Closed("the deck stopped answering ping".into()))
        );
        assert!(!was_open);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "heartbeat pipe broke",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct TimeoutReader;

    impl Read for TimeoutReader {
        fn read(&mut self, _bytes: &mut [u8]) -> io::Result<usize> {
            thread::sleep(crate::READ_TICK);
            Err(ErrorKind::TimedOut.into())
        }
    }

    struct BrokenTransport;

    impl Transport for BrokenTransport {
        fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
            Ok((Box::new(TimeoutReader), Box::new(BrokenWriter)))
        }
    }

    #[test]
    fn heartbeat_write_failure_closes_the_session() {
        let config = SessionConfig {
            ping_every: Some(Duration::from_millis(10)),
            ping_timeout: Duration::from_millis(100),
            close_on_restart: false,
            close_on_drop: false,
        };
        let (session, events) = Session::open(Box::new(BrokenTransport), config).unwrap();
        let closed = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            closed,
            Incoming::Closed("heartbeat failed: write failed: heartbeat pipe broke".into())
        );
        assert!(!session.is_open());
        assert!(matches!(
            session.request(&Request::Hello, Duration::from_millis(50)),
            Err(LinkError::Closed(_))
        ));
        session.close();
        assert!(events.try_recv().is_err(), "only one Closed event");
    }

    #[test]
    fn closing_and_dropping_after_a_writer_panic_does_not_abort() {
        struct PanicWriter;

        impl Write for PanicWriter {
            fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
                panic!("writer panicked");
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        struct PanicTransport;

        impl Transport for PanicTransport {
            fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
                Ok((Box::new(TimeoutReader), Box::new(PanicWriter)))
            }
        }

        let config = SessionConfig {
            ping_every: Some(Duration::from_millis(10)),
            ping_timeout: Duration::from_millis(100),
            close_on_restart: false,
            close_on_drop: false,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (session, _events) = Session::open(Box::new(PanicTransport), config).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while !session.shared.writer.is_poisoned() {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
            session.close();
            panic!("caller unwinds with a poisoned writer");
        }));
        assert!(result.is_err());
    }

    #[test]
    fn missed_pings_close_the_session() {
        let deck = MockDeck::new();
        let config = SessionConfig {
            ping_every: Some(Duration::from_millis(100)),
            ping_timeout: Duration::from_millis(100),
            close_on_restart: false,
            close_on_drop: false,
        };
        let (session, events) = Session::open(Box::new(deck.connect()), config).unwrap();
        session.hello(Duration::from_secs(1)).unwrap();
        deck.stop();
        let reason = until(&events, |item| match item {
            Incoming::Closed(reason) => Some(reason),
            _ => None,
        });
        assert_eq!(reason, "the deck stopped answering ping");
        assert!(!session.is_open());
        assert!(matches!(
            session.request(&Request::Hello, Duration::from_millis(50)),
            Err(LinkError::Closed(_))
        ));
    }

    #[test]
    fn a_hang_up_closes_the_session_at_once() {
        let deck = MockDeck::new();
        let (session, events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        deck.hang_up();
        let reason = until(&events, |item| match item {
            Incoming::Closed(reason) => Some(reason),
            _ => None,
        });
        assert_eq!(reason, "the deck closed the stream");
        assert!(!session.is_open());
    }

    #[test]
    fn a_restart_closes_the_session_only_when_asked() {
        for close_on_restart in [true, false] {
            let deck = MockDeck::new();
            deck.set_uptime(Duration::from_secs(10));
            let config = SessionConfig {
                ping_every: Some(Duration::from_millis(50)),
                ping_timeout: Duration::from_millis(500),
                close_on_restart,
                close_on_drop: false,
            };
            let (session, events) = Session::open(Box::new(deck.connect()), config).unwrap();
            let pings = || {
                deck.received()
                    .iter()
                    .filter(|request| request["m"] == "ping")
                    .count()
            };
            let pinged = |count: usize| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while pings() < count {
                    assert!(Instant::now() < deadline, "the session stopped pinging");
                    thread::sleep(Duration::from_millis(1));
                }
            };
            pinged(2);
            session.send_line("reboot").unwrap();
            if close_on_restart {
                let reason = until(&events, |item| match item {
                    Incoming::Closed(reason) => Some(reason),
                    _ => None,
                });
                assert_eq!(reason, "the deck restarted");
            } else {
                pinged(pings() + 3);
                assert!(session.is_open());
            }
        }
    }

    fn watching(ping_every: Option<Duration>, close_on_drop: bool) -> SessionConfig {
        SessionConfig {
            ping_every,
            ping_timeout: Duration::from_millis(500),
            close_on_restart: false,
            close_on_drop,
        }
    }

    fn closed_within(events: &IncomingReceiver, wait: Duration) -> Option<String> {
        let deadline = Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match events.recv_timeout(left) {
                Ok(Incoming::Closed(reason)) => return Some(reason),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    }

    #[test]
    fn the_heartbeat_closes_a_session_the_deck_dropped_only_when_asked() {
        for close_on_drop in [true, false] {
            let deck = MockDeck::v2();
            let config = watching(Some(Duration::from_millis(50)), close_on_drop);
            let (session, events) = Session::open(Box::new(deck.connect()), config).unwrap();
            assert_eq!(
                session.identity(Duration::from_secs(1)).unwrap().protocol,
                2
            );
            thread::sleep(Duration::from_millis(200));
            assert!(session.is_open());
            deck.drop_session();
            let closed = closed_within(&events, Duration::from_secs(1));
            if close_on_drop {
                assert_eq!(closed.as_deref(), Some(DROPPED));
                assert!(!session.is_open());
            } else {
                assert_eq!(closed, None);
                assert!(session.is_open());
            }
        }
    }

    #[test]
    fn a_not_negotiated_refusal_after_protocol_2_closes_the_session_at_once() {
        let deck = MockDeck::v2();
        let (session, events) =
            Session::open(Box::new(deck.connect()), watching(None, true)).unwrap();
        session.identity(Duration::from_secs(1)).unwrap();
        assert!(
            session
                .call(
                    &serde_json::json!({"m":"v2.name.get"}),
                    Duration::from_secs(1)
                )
                .unwrap()
                .ok
        );
        deck.drop_session();
        let refused = session
            .call(
                &serde_json::json!({"m":"v2.name.get"}),
                Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(refused.fields["reason"], "not-negotiated");
        assert_eq!(
            closed_within(&events, Duration::from_secs(1)).as_deref(),
            Some(DROPPED)
        );
        assert!(matches!(
            session.hello(Duration::from_millis(100)),
            Err(LinkError::Closed(reason)) if reason == DROPPED
        ));
    }

    #[test]
    fn a_not_negotiated_refusal_before_protocol_2_keeps_the_session() {
        let deck = MockDeck::v2();
        let (session, events) =
            Session::open(Box::new(deck.connect()), watching(None, true)).unwrap();
        assert!(session.hello(Duration::from_secs(1)).unwrap().is_deck());
        let refused = session
            .call(
                &serde_json::json!({"m":"v2.name.get"}),
                Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(refused.fields["reason"], "not-negotiated");
        assert_eq!(closed_within(&events, Duration::from_millis(200)), None);
        assert!(session.is_open());
    }

    #[test]
    fn console_lines_and_their_replies_arrive_as_text() {
        let deck = MockDeck::new();
        let (session, events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        session.send_line("version").unwrap();
        let reply = until(&events, |item| match item {
            Incoming::Line(line) if line.contains("@td ") => Some(line),
            _ => None,
        });
        let json: Value = serde_json::from_str(&reply[reply.find("@td ").unwrap() + 4..]).unwrap();
        assert_eq!(
            (json["cmd"].as_str(), json["ok"].as_bool()),
            (Some("version"), Some(true))
        );
        assert_eq!(deck.lines(), ["version"]);
    }

    #[test]
    fn layout_state_can_be_read_from_an_event_handler_without_consuming_console_text() {
        let deck = MockDeck::new();
        let (session, events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        deck.emit_press("shared", "feed_land", "widget");
        until(&events, |item| {
            matches!(item, Incoming::Event(_)).then_some(())
        });
        // Like the app's event handler, do not drain Incoming while waiting for state.
        let state = session.layout_state(Duration::from_secs(1)).unwrap();
        assert_eq!(state, deck.layout_state());
        let line = until(&events, |item| match item {
            Incoming::Line(line) if line.contains("@td ") => Some(line),
            _ => None,
        });
        assert!(line.contains("\"op\":\"state\""));
        assert_eq!(deck.lines(), ["layout state"]);
        assert!(session.hello(Duration::from_secs(1)).unwrap().is_deck());
    }

    #[test]
    fn a_layout_state_timeout_releases_the_console_and_the_next_query_can_succeed() {
        let deck = MockDeck::new();
        deck.delay_replies(Duration::from_millis(80));
        let (session, events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        assert_eq!(
            session.layout_state(Duration::from_millis(10)),
            Err(LinkError::Timeout(Duration::from_millis(10)))
        );
        assert_eq!(
            session.layout_state(Duration::from_secs(1)),
            Err(LinkError::ConsoleBusy)
        );
        until(&events, |item| {
            matches!(item, Incoming::Line(_)).then_some(())
        });
        deck.delay_replies(Duration::ZERO);
        session.send_line("version").unwrap();
        assert_eq!(
            session.layout_state(Duration::from_secs(1)).unwrap(),
            deck.layout_state()
        );
    }

    #[test]
    fn closing_the_connection_wakes_a_layout_state_waiter() {
        let deck = MockDeck::new();
        deck.lose_console_replies(1);
        let (session, _events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        let session = Arc::new(session);
        let query = {
            let session = session.clone();
            thread::spawn(move || session.layout_state(Duration::from_secs(5)))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while deck.lines().is_empty() {
            assert!(Instant::now() < deadline, "the query did not start");
            thread::sleep(Duration::from_millis(1));
        }
        session.close();
        let result = query.join().unwrap();
        assert!(matches!(result, Err(LinkError::Closed(_))), "{result:?}");
    }

    #[test]
    fn a_layout_state_query_after_close_returns_closed() {
        let deck = MockDeck::new();
        let (session, _events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        session.close();
        assert!(matches!(
            session.layout_state(Duration::from_secs(5)),
            Err(LinkError::Closed(_))
        ));
    }

    #[test]
    fn a_lost_layout_state_reply_can_be_replaced_after_the_stale_deadline() {
        let deck = MockDeck::new();
        deck.lose_console_replies(1);
        let (session, _events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        assert!(matches!(
            session.layout_state(Duration::from_millis(10)),
            Err(LinkError::Timeout(_))
        ));
        assert_eq!(
            session.layout_state(Duration::from_secs(1)),
            Err(LinkError::ConsoleBusy)
        );
        thread::sleep(Duration::from_millis(3100));
        assert_eq!(
            session.layout_state(Duration::from_secs(1)).unwrap(),
            deck.layout_state()
        );
    }

    #[test]
    fn a_console_input_overflow_clears_the_layout_state_waiter() {
        let deck = MockDeck::new();
        deck.lose_console_replies(1);
        let (session, _events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        assert!(matches!(
            session.layout_state(Duration::from_millis(10)),
            Err(LinkError::Timeout(_))
        ));
        session.shared.route(
            Item::Text(
                b"@td {\"cmd\":\"console\",\"ok\":false,\"error\":\"input overflow\"}\r\n".to_vec(),
            ),
            &mut ConsoleLine::default(),
        );
        assert_eq!(
            session.layout_state(Duration::from_secs(1)).unwrap(),
            deck.layout_state()
        );
    }

    #[test]
    fn icon_lists_page_32_at_a_time() {
        let deck = MockDeck::new();
        deck.add_icons((0..128).map(|i| format!("{i:016x}")));
        let (session, _events) = Session::open(Box::new(deck.connect()), quiet()).unwrap();
        let (mut ids, mut cursor, mut pages) = (Vec::new(), None, 0);
        loop {
            let page: IconPage = session
                .request(&Request::IconList { cursor }, Duration::from_secs(1))
                .unwrap()
                .fields()
                .unwrap();
            assert!(page.ids.len() <= td_core::ICON_PAGE);
            ids.extend(page.ids);
            pages += 1;
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!((ids.len(), pages), (128, 4));
        assert_eq!(
            ids,
            (0..128).map(|i| format!("{i:016x}")).collect::<Vec<_>>()
        );
    }
}
