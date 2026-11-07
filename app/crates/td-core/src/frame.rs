use std::fmt;
use std::time::{Duration, Instant};

pub const VERSION: u8 = 1;
pub const PAYLOAD_MAX: usize = 1024;
pub const BODY_MAX: usize = PAYLOAD_MAX + 8;
pub const WIRE_MAX: usize = 2 * BODY_MAX + 2;
pub const CHUNK_DATA_MAX: usize = PAYLOAD_MAX - 3;
pub const FRAME_DEADLINE: Duration = Duration::from_millis(500);

const END: u8 = 0xC0;
const ESC: u8 = 0xDB;
const ESC_END: u8 = 0xDC;
const ESC_ESC: u8 = 0xDD;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Request(u8),
    Response(u8),
    Event,
}

impl Role {
    pub fn seq(self) -> u8 {
        match self {
            Role::Request(seq) | Role::Response(seq) => seq,
            Role::Event => 0,
        }
    }

    pub fn flags(self) -> u8 {
        match self {
            Role::Request(_) => 0,
            Role::Response(_) => 1,
            Role::Event => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Message(Vec<u8>),
    Chunk { xfer: u8, idx: u16, data: Vec<u8> },
    Ack { xfer: u8, next: u16 },
}

impl Body {
    pub fn kind(&self) -> u8 {
        match self {
            Body::Message(_) => 1,
            Body::Chunk { .. } => 2,
            Body::Ack { .. } => 3,
        }
    }

    pub fn payload(&self) -> Vec<u8> {
        match self {
            Body::Message(payload) => payload.clone(),
            Body::Chunk { xfer, idx, data } => {
                let mut payload = vec![*xfer];
                payload.extend_from_slice(&idx.to_le_bytes());
                payload.extend_from_slice(data);
                payload
            }
            Body::Ack { xfer, next } => {
                let mut payload = vec![*xfer];
                payload.extend_from_slice(&next.to_le_bytes());
                payload
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub role: Role,
    pub body: Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    Length,
    Version,
    Crc,
    Header,
    Escape,
}

impl FrameError {
    pub fn as_str(self) -> &'static str {
        match self {
            FrameError::Length => "length",
            FrameError::Version => "version",
            FrameError::Crc => "crc",
            FrameError::Header => "header",
            FrameError::Escape => "escape",
        }
    }
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for FrameError {}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

impl Frame {
    pub fn message(role: Role, payload: Vec<u8>) -> Frame {
        Frame {
            role,
            body: Body::Message(payload),
        }
    }

    /// The frame body without SLIP: header, payload and CRC, as a network record carries it.
    pub fn encode_body(&self) -> Result<Vec<u8>, FrameError> {
        let payload = self.body.payload();
        if payload.len() > PAYLOAD_MAX {
            return Err(FrameError::Length);
        }
        let mut body = Vec::with_capacity(payload.len() + 8);
        body.extend_from_slice(&[
            VERSION,
            self.body.kind(),
            self.role.seq(),
            self.role.flags(),
        ]);
        body.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        body.extend_from_slice(&payload);
        let crc = crc16(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        Ok(body)
    }

    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        let body = self.encode_body()?;
        let mut wire = Vec::with_capacity(2 * body.len() + 2);
        wire.push(END);
        for byte in body {
            match byte {
                END => wire.extend_from_slice(&[ESC, ESC_END]),
                ESC => wire.extend_from_slice(&[ESC, ESC_ESC]),
                _ => wire.push(byte),
            }
        }
        wire.push(END);
        Ok(wire)
    }

    pub fn decode_body(body: &[u8]) -> Result<Frame, FrameError> {
        if body.len() < 8 || body.len() > BODY_MAX {
            return Err(FrameError::Length);
        }
        if body[0] != VERSION {
            return Err(FrameError::Version);
        }
        let len = usize::from(u16::from_le_bytes([body[4], body[5]]));
        if len > PAYLOAD_MAX || body.len() != len + 8 {
            return Err(FrameError::Length);
        }
        let crc = u16::from_le_bytes([body[len + 6], body[len + 7]]);
        if crc16(&body[..len + 6]) != crc {
            return Err(FrameError::Crc);
        }
        let (kind, seq, flags) = (body[1], body[2], body[3]);
        let payload = &body[6..6 + len];
        let role = match (flags, seq) {
            (0, seq) => Role::Request(seq),
            (1, seq) => Role::Response(seq),
            (2, 0) => Role::Event,
            _ => return Err(FrameError::Header),
        };
        let body = match (kind, payload.len()) {
            (1, _) => Body::Message(payload.to_vec()),
            (2, 3..) => Body::Chunk {
                xfer: payload[0],
                idx: u16::from_le_bytes([payload[1], payload[2]]),
                data: payload[3..].to_vec(),
            },
            (3, 3) => Body::Ack {
                xfer: payload[0],
                next: u16::from_le_bytes([payload[1], payload[2]]),
            },
            _ => return Err(FrameError::Header),
        };
        Ok(Frame { role, body })
    }

    pub fn decode_wire(wire: &[u8]) -> Result<Frame, FrameError> {
        if wire.len() < 2 || wire[0] != END || wire[wire.len() - 1] != END {
            return Err(FrameError::Length);
        }
        let mut body = Vec::with_capacity(wire.len());
        let mut escaped = false;
        for &byte in &wire[1..wire.len() - 1] {
            match (escaped, byte) {
                (_, END) => return Err(FrameError::Escape),
                (true, ESC_END) => body.push(END),
                (true, ESC_ESC) => body.push(ESC),
                (true, _) => return Err(FrameError::Escape),
                (false, ESC) => {
                    escaped = true;
                    continue;
                }
                (false, _) => body.push(byte),
            }
            escaped = false;
        }
        if escaped {
            return Err(FrameError::Escape);
        }
        Frame::decode_body(&body)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Text(Vec<u8>),
    Frame(Frame),
}

fn declared_len(body: &[u8]) -> usize {
    usize::from(u16::from_le_bytes([body[4], body[5]]))
}

fn header_valid(header: &[u8]) -> bool {
    let len = declared_len(header);
    let kind_fits = match header[1] {
        1 => true,
        2 => len >= 3,
        3 => len == 3,
        _ => false,
    };
    header[0] == VERSION
        && kind_fits
        && header[3] <= 2
        && !(header[3] == 2 && header[2] != 0)
        && len <= PAYLOAD_MAX
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Closed,
    Body,
    Escape,
}

/// Splits frames from console text on the PC side.
///
/// A delimiter opens a frame only when `ver` follows it, and so does a `ver` right after a
/// closed frame, so a lost closing delimiter costs one frame at most. A frame closes only
/// when its body decodes, and a body of the right length that fails to decode makes its
/// closing delimiter the next opener. Once the header is in, a bad header or a body longer
/// than `len` plus 8 returns its raw bytes to the text at once, and so does a frame still
/// open [`FRAME_DEADLINE`] after its opener, so a console reply or log line never waits
/// inside a dead frame.
#[derive(Debug)]
pub struct StreamDecoder {
    mode: Mode,
    body: Vec<u8>,
    raw: Vec<u8>,
    opened: Option<Instant>,
    errors: u64,
}

impl Default for StreamDecoder {
    fn default() -> Self {
        StreamDecoder {
            mode: Mode::Text,
            body: Vec::new(),
            raw: Vec::new(),
            opened: None,
            errors: 0,
        }
    }
}

impl StreamDecoder {
    pub fn errors(&self) -> u64 {
        self.errors
    }

    fn open(&mut self, now: Instant) {
        self.mode = Mode::Body;
        self.body.clear();
        self.raw.clear();
        self.opened = Some(now);
    }

    fn abandon(&mut self, text: &mut Vec<u8>) {
        self.errors += 1;
        text.append(&mut self.raw);
        self.body.clear();
        self.mode = Mode::Text;
    }

    fn overrun(&self) -> bool {
        match self.body.len() {
            6 => !header_valid(&self.body),
            n => n > 6 && n > declared_len(&self.body) + 8,
        }
    }

    fn complete(&self) -> bool {
        self.mode == Mode::Body
            && self.body.len() >= 8
            && self.body.len() == declared_len(&self.body) + 8
    }

    fn take(&mut self, byte: u8, text: &mut Vec<u8>) {
        self.raw.push(byte);
        match (self.mode, byte) {
            (Mode::Escape, ESC_END) => self.body.push(END),
            (Mode::Escape, ESC_ESC) => self.body.push(ESC),
            (Mode::Escape, _) => return self.abandon(text),
            (_, ESC) => {
                self.mode = Mode::Escape;
                return;
            }
            _ => self.body.push(byte),
        }
        self.mode = Mode::Body;
        if self.overrun() {
            self.abandon(text);
        }
    }

    /// Returns a frame still open [`FRAME_DEADLINE`] after its opener to the text stream.
    pub fn poll(&mut self, now: Instant) -> Vec<Item> {
        let expired = self
            .opened
            .is_some_and(|opened| now.duration_since(opened) >= FRAME_DEADLINE);
        if !matches!(self.mode, Mode::Body | Mode::Escape) || !expired {
            return Vec::new();
        }
        self.mode = Mode::Text;
        self.body.clear();
        if self.raw.is_empty() {
            return Vec::new();
        }
        self.errors += 1;
        vec![Item::Text(std::mem::take(&mut self.raw))]
    }

    pub fn feed(&mut self, data: &[u8], now: Instant) -> Vec<Item> {
        let mut items = self.poll(now);
        let mut text = Vec::new();
        for &byte in data {
            if byte == END {
                if self.complete() {
                    match Frame::decode_body(&self.body) {
                        Ok(frame) => {
                            if !text.is_empty() {
                                items.push(Item::Text(std::mem::take(&mut text)));
                            }
                            items.push(Item::Frame(frame));
                            self.body.clear();
                            self.raw.clear();
                            self.mode = Mode::Closed;
                        }
                        Err(_) => {
                            self.errors += 1;
                            self.open(now);
                        }
                    }
                    continue;
                }
                if matches!(self.mode, Mode::Body | Mode::Escape) && !self.raw.is_empty() {
                    self.abandon(&mut text);
                }
                if !text.is_empty() {
                    items.push(Item::Text(std::mem::take(&mut text)));
                }
                self.open(now);
                continue;
            }
            match self.mode {
                Mode::Text => text.push(byte),
                Mode::Closed | Mode::Body if self.raw.is_empty() && byte != VERSION => {
                    self.mode = Mode::Text;
                    text.push(byte);
                }
                Mode::Closed => {
                    self.open(now);
                    self.take(byte, &mut text);
                }
                Mode::Body | Mode::Escape => self.take(byte, &mut text),
            }
        }
        if !text.is_empty() {
            items.push(Item::Text(text));
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn limits_match_the_protocol() {
        assert_eq!((BODY_MAX, WIRE_MAX, CHUNK_DATA_MAX), (1032, 2066, 1021));
    }

    #[test]
    fn encoder_rejects_oversize_payloads() {
        let message = Frame::message(Role::Request(1), vec![0; PAYLOAD_MAX + 1]);
        assert_eq!(message.encode(), Err(FrameError::Length));
        let chunk = Frame {
            role: Role::Request(1),
            body: Body::Chunk {
                xfer: 1,
                idx: 0,
                data: vec![0; CHUNK_DATA_MAX + 1],
            },
        };
        assert_eq!(chunk.encode(), Err(FrameError::Length));
    }

    #[test]
    fn event_with_a_sequence_is_rejected() {
        let mut body = vec![VERSION, 1, 7, 2, 0, 0];
        let crc = crc16(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        assert_eq!(Frame::decode_body(&body), Err(FrameError::Header));
    }

    fn split(stream: &[u8]) -> (Vec<u8>, Vec<Frame>, u64) {
        let mut decoder = StreamDecoder::default();
        let (mut text, mut frames) = (Vec::new(), Vec::new());
        for item in decoder.feed(stream, Instant::now()) {
            match item {
                Item::Text(bytes) => text.extend(bytes),
                Item::Frame(frame) => frames.push(frame),
            }
        }
        (text, frames, decoder.errors())
    }

    fn ping(seq: u8) -> Frame {
        Frame::message(Role::Request(seq), br#"{"m":"ping","n":0}"#.to_vec())
    }

    #[test]
    fn every_frame_carries_both_delimiters() {
        let one = ping(1).encode().unwrap();
        assert_eq!((one[0], one[one.len() - 1]), (END, END));
        let mut stream = one.clone();
        stream.extend(ping(2).encode().unwrap());
        assert_eq!(split(&stream), (vec![], vec![ping(1), ping(2)], 0));
    }

    #[test]
    fn a_delimiter_not_followed_by_ver_opens_nothing() {
        let mut stream = ping(1).encode().unwrap();
        stream.push(END);
        stream.extend(b"version\n");
        assert_eq!(split(&stream), (b"version\n".to_vec(), vec![ping(1)], 0));
        assert_eq!(
            split(b"\xc0\xc0\xc0log line\n"),
            (b"log line\n".to_vec(), vec![], 0)
        );
    }

    #[test]
    fn a_right_length_garbage_body_hands_its_closer_to_the_next_frame() {
        let mut garbage = ping(9).encode().unwrap();
        let crc_low = garbage.len() - 3;
        garbage[crc_low] ^= 0x55;
        let mut stream = garbage.clone();
        stream.extend(ping(1).encode().unwrap());
        assert_eq!(split(&stream), (vec![], vec![ping(1)], 1));
        let mut shared = garbage;
        shared.extend(&ping(1).encode().unwrap()[1..]);
        assert_eq!(split(&shared), (vec![], vec![ping(1)], 1));
    }

    #[test]
    fn a_cut_frame_gives_a_following_console_reply_back_at_once() {
        let wire = ping(1).encode().unwrap();
        let reply = b"td> @td {\"cmd\":\"version\",\"ok\":true}\r\n";
        let mut stream = wire[..10].to_vec();
        stream.extend(reply);
        let (text, frames, errors) = split(&stream);
        assert!(
            text.ends_with(reply),
            "{:?}",
            String::from_utf8_lossy(&text)
        );
        assert_eq!((frames, errors), (vec![], 1));
    }

    #[test]
    fn a_bad_header_gives_its_bytes_back_after_six() {
        let mut stream = vec![END, 0x01, 0x09, 0x00, 0x00, 0x00, 0x00];
        stream.extend(b"I (5) touchdeck: alive 2\n");
        assert_eq!(split(&stream), (stream[1..].to_vec(), vec![], 1));
    }

    #[test]
    fn a_stray_delimiter_before_long_text_keeps_the_text() {
        let mut stream = vec![END];
        stream.extend(std::iter::repeat_n(b'x', WIRE_MAX + 1));
        stream.push(b'\n');
        assert_eq!(split(&stream), (stream[1..].to_vec(), vec![], 0));
    }

    #[test]
    fn ver_right_after_a_closed_frame_opens_the_next_frame() {
        let first = ping(1).encode().unwrap();
        let mut stream = first[..first.len() - 1].to_vec();
        stream.extend(ping(2).encode().unwrap());
        assert_eq!(split(&stream), (vec![], vec![ping(1), ping(2)], 0));
        let mut shared = first.clone();
        shared.extend(&ping(2).encode().unwrap()[1..]);
        assert_eq!(split(&shared), (vec![], vec![ping(1), ping(2)], 0));
        let mut text = first;
        text.extend(b"log\n");
        assert_eq!(split(&text), (b"log\n".to_vec(), vec![ping(1)], 0));
    }

    #[test]
    fn a_frame_still_open_after_the_deadline_returns_to_text() {
        let start = Instant::now();
        let mut decoder = StreamDecoder::default();
        let mut cut = vec![END, 0x01, 0x01, 0x05, 0x00, 0xE8, 0x03];
        cut.extend(b"{\"m\"");
        assert_eq!(decoder.feed(&cut, start), vec![]);
        let mut held = Vec::new();
        for step in 1..=4u64 {
            let at = start + Duration::from_millis(100 * step);
            assert_eq!(
                decoder.feed(format!("line {step}\n").as_bytes(), at),
                vec![],
                "held before the deadline"
            );
            held.extend(format!("line {step}\n").into_bytes());
        }
        assert_eq!(decoder.poll(start + Duration::from_millis(499)), vec![]);
        let mut expected = cut[1..].to_vec();
        expected.extend(held);
        assert_eq!(
            decoder.poll(start + FRAME_DEADLINE),
            vec![Item::Text(expected)]
        );
        assert_eq!(decoder.errors(), 1);
        let late = start + Duration::from_millis(600);
        assert_eq!(
            decoder.feed(b"line 6\n", late),
            vec![Item::Text(b"line 6\n".to_vec())]
        );
    }

    #[test]
    fn the_deadline_runs_from_the_opener_even_inside_one_feed() {
        let start = Instant::now();
        let mut decoder = StreamDecoder::default();
        decoder.feed(&[END, 0x01, 0x01, 0x05, 0x00, 0xE8, 0x03], start);
        let items = decoder.feed(b"version reply\n", start + Duration::from_millis(700));
        assert_eq!(
            items,
            vec![
                Item::Text(vec![0x01, 0x01, 0x05, 0x00, 0xE8, 0x03]),
                Item::Text(b"version reply\n".to_vec())
            ]
        );
        assert_eq!(decoder.poll(start + Duration::from_secs(5)), vec![]);
    }
}
