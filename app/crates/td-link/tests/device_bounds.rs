use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};
use td_core::{Body, Frame, Item, Role, StreamDecoder};
use td_link::{Incoming, Session, SessionConfig, Transport};

struct Reader {
    rx: mpsc::Receiver<Vec<u8>>,
    bytes: VecDeque<u8>,
    flood: Option<(Instant, Vec<u8>)>,
}
impl Read for Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Ok(reply) = self.rx.try_recv() {
            for byte in reply.into_iter().rev() {
                self.bytes.push_front(byte);
            }
        }
        if self.bytes.is_empty() {
            match self.rx.recv_timeout(Duration::from_millis(1)) {
                Ok(bytes) => self.bytes.extend(bytes),
                Err(_) => match &self.flood {
                    Some((until, bytes)) if Instant::now() < *until => self.bytes.extend(bytes),
                    _ => return Err(io::ErrorKind::TimedOut.into()),
                },
            }
        }
        let n = out.len().min(self.bytes.len());
        for byte in &mut out[..n] {
            *byte = self.bytes.pop_front().unwrap();
        }
        Ok(n)
    }
}
struct Writer {
    tx: mpsc::Sender<Vec<u8>>,
    decoder: StreamDecoder,
    reply: Box<dyn FnMut(Value) -> Value + Send>,
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for item in self.decoder.feed(bytes, Instant::now()) {
            if let Item::Frame(Frame {
                role: Role::Request(seq),
                body: Body::Message(data),
            }) = item
            {
                let req: Value = serde_json::from_slice(&data).unwrap();
                let mut reply = if req["m"] == "ping" {
                    json!({"t": 1234, "n": req["n"]})
                } else {
                    (self.reply)(req.clone())
                };
                reply["m"] = req["m"].clone();
                reply["ok"] = json!(true);
                self.tx
                    .send(
                        Frame::message(Role::Response(seq), serde_json::to_vec(&reply).unwrap())
                            .encode()
                            .unwrap(),
                    )
                    .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Fake(Reader, Writer);
impl Transport for Fake {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        Ok((Box::new(self.0), Box::new(self.1)))
    }
}
fn open(
    initial: Vec<u8>,
    flood: Option<Vec<u8>>,
    reply: impl FnMut(Value) -> Value + Send + 'static,
) -> (Session, td_link::IncomingReceiver) {
    let (tx, rx) = mpsc::channel();
    Session::open(
        Box::new(Fake(
            Reader {
                rx,
                bytes: initial.into(),
                flood: flood.map(|v| (Instant::now() + Duration::from_secs(4), v)),
            },
            Writer {
                tx,
                decoder: StreamDecoder::default(),
                reply: Box::new(reply),
            },
        )),
        SessionConfig {
            ping_every: Some(Duration::from_millis(20)),
            ping_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn console_overflow_recovers_and_keeps_replies_working() {
    let mut input = vec![b'x'; 8 * 1024 * 1024];
    input.extend(b"\nreal line\n");
    let (session, events) = open(input, None, |_| json!({}));
    let Incoming::Line(line) = events.recv_timeout(Duration::from_secs(5)).unwrap() else {
        panic!()
    };
    assert!(
        line.contains("truncated") && line.len() < 1024,
        "line length {}",
        line.len()
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        Incoming::Line("real line".into())
    );
    assert!(
        session
            .call(&json!({"m":"ping"}), Duration::from_secs(1))
            .is_ok()
    );
    session.close();
}
#[test]
fn incoming_queue_is_bounded_without_blocking_replies() {
    let (session, events) = open(b"line\n".repeat(10000), None, |_| json!({}));
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        session
            .call(&json!({"m":"ping"}), Duration::from_secs(1))
            .is_ok()
    );
    assert!(events.try_iter().count() <= 256);
    session.close();
}
fn pager(legacy: bool, mode: u8) {
    let mut n = 0;
    let (session, _) = open(vec![], None, move |_| {
        n += 1;
        let next = if n == 10000 {
            Value::Null
        } else if legacy {
            json!(if mode == 0 { n % 2 } else { n })
        } else {
            json!(format!("{:064x}", if mode == 0 { n % 2 } else { n }))
        };
        if legacy {
            json!({"ids":[format!("{:016x}", if mode == 1 { 0 } else { n })], "next":next})
        } else {
            json!({"items":[{"id":format!("{:064x}", if mode == 1 { 0 } else { n }),"bytes":1,"format":"tdi1"}], "next":next})
        }
    });
    let result = if legacy {
        td_link::xfer::list_icons(&session).map(|_| ())
    } else {
        td_link::asset::list_assets(&session)
            .map(|_| ())
            .map_err(|e| td_link::xfer::XferError::Link(td_link::LinkError::Io(e.to_string())))
    };
    session.close();
    assert!(result.is_err());
}
#[test]
fn alternating_asset_cursors_fail() {
    pager(false, 0);
}
#[test]
fn repeated_assets_fail() {
    pager(false, 1);
}
#[test]
fn endless_asset_pages_fail() {
    pager(false, 2);
}
#[test]
fn alternating_icon_cursors_fail() {
    pager(true, 0);
}
#[test]
fn repeated_icons_fail() {
    pager(true, 1);
}
#[test]
fn endless_icon_pages_fail() {
    pager(true, 2);
}
#[test]
fn irrelevant_chunks_do_not_keep_a_get_alive() {
    let flood = Frame {
        role: Role::Request(1),
        body: Body::Chunk {
            xfer: 99,
            idx: 0,
            data: vec![1],
        },
    }
    .encode()
    .unwrap();
    let (session, _) = open(
        vec![],
        Some(flood),
        |_| json!({"xfer":1,"bytes":100,"crc32":0}),
    );
    let start = Instant::now();
    assert!(td_link::xfer::get_layout(&session, td_core::Orientation::Landscape).is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(
        !session.is_open(),
        "an abandoned transfer closes Device state"
    );
    session.close();
}
#[test]
fn irrelevant_acks_do_not_keep_a_put_alive() {
    let flood = Frame {
        role: Role::Response(1),
        body: Body::Ack { xfer: 99, next: 0 },
    }
    .encode()
    .unwrap()
    .repeat(256);
    let (session, _) = open(vec![], Some(flood), |_| json!({"xfer":1}));
    let start = Instant::now();
    assert!(
        td_link::xfer::put_icon(
            &session,
            "0000000000000001",
            &vec![0; td_link::xfer::ICON_BYTES]
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    session.close();
}

#[test]
fn a_full_incoming_queue_still_delivers_close() {
    let (session, events) = open(b"line\n".repeat(10000), None, |_| json!({}));
    std::thread::sleep(Duration::from_millis(200));
    session.close();
    assert!(
        events
            .try_iter()
            .any(|item| matches!(item, Incoming::Closed(_)))
    );
}

#[test]
fn duplicate_chunks_do_not_keep_a_get_alive() {
    let flood = Frame {
        role: Role::Request(1),
        body: Body::Chunk {
            xfer: 1,
            idx: 0,
            data: vec![1; 1021],
        },
    }
    .encode()
    .unwrap();
    let (session, _) = open(
        vec![],
        Some(flood),
        |_| json!({"xfer":1,"bytes":2042,"crc32":0}),
    );
    let start = Instant::now();
    assert!(td_link::xfer::get_layout(&session, td_core::Orientation::Landscape).is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(
        !session.is_open(),
        "an abandoned transfer closes Device state"
    );
    assert!(session.transfer().is_err());
    session.close();
}

#[test]
fn duplicate_acks_do_not_keep_a_put_alive() {
    let flood = Frame {
        role: Role::Response(1),
        body: Body::Ack { xfer: 1, next: 0 },
    }
    .encode()
    .unwrap()
    .repeat(256);
    let (session, _) = open(vec![], Some(flood), |_| json!({"xfer":1}));
    let start = Instant::now();
    assert!(
        td_link::xfer::put_icon(
            &session,
            "0000000000000001",
            &vec![0; td_link::xfer::ICON_BYTES]
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(
        !session.is_open(),
        "an abandoned transfer closes Device state"
    );
    assert!(session.transfer().is_err());
    session.close();
}
