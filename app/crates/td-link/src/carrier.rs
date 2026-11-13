//! The seam under a link [`Session`](crate::Session): a carrier moves whole link frames
//! between the Companion and one Device. Hello, protocol selection, ping and expiry,
//! transfers and Live values stay in the Session, so they work the same over every carrier.
//!
//! [`UsbCarrier`] is the first carrier: SLIP frames mixed with console text on a byte
//! stream. A carrier that has frames of its own (records over TCP, GATT writes) skips the
//! byte framing and hands the Session each frame as it arrives.

use std::io::{self, ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use td_core::{Frame, Item, StreamDecoder};

use crate::Transport;

/// One Device link that moves whole frames. [`split`](Carrier::split) is called once, when
/// the Session opens; the reader then runs on the Session's read thread and the writer is
/// shared by every sender behind a lock.
pub trait Carrier: Send {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn FrameReader>, Box<dyn FrameWriter>)>;
}

/// The receiving half of a [`Carrier`].
pub trait FrameReader: Send {
    /// Waits about [`READ_TICK`](crate::READ_TICK) for what the Device sent, so the Session
    /// notices a close. An empty [`Received::Items`] means nothing arrived. An error ends the
    /// Session with `read failed: {error}`.
    fn recv(&mut self) -> io::Result<Received>;
}

/// The sending half of a [`Carrier`].
pub trait FrameWriter: Send {
    /// Checks whether a new operation may start. A transfer calls this once before its
    /// begin request; its chunks, heartbeat and completion share that admission.
    fn start_operation(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Sends one frame or console line, or fails within `window`. A carrier that sends in
    /// pieces checks the window between them, so a closed Session stops the send at once.
    /// A carrier without a console refuses [`Outbound::Line`] with `Unsupported`.
    fn send(&mut self, out: Outbound<'_>, window: &SendWindow<'_>) -> io::Result<()>;
}

/// What one [`FrameReader::recv`] brought, in arrival order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    /// Frames and console text. Console text may split anywhere; the Session joins lines.
    Items(Vec<Item>),
    /// The Device closed the link. The Session closes with "the deck closed the stream".
    Ended,
}

/// What the Session sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outbound<'a> {
    /// A link frame. The Session has already checked that its payload fits one frame.
    Frame(&'a Frame),
    /// One console line, without its line ending. It holds no CR or LF.
    Line(&'a str),
}

/// How long a send may take, and whether the Session is still open for it.
#[derive(Debug, Clone, Copy)]
pub struct SendWindow<'a> {
    deadline: Instant,
    open: &'a AtomicBool,
}

impl<'a> SendWindow<'a> {
    pub(crate) fn new(deadline: Instant, open: &'a AtomicBool) -> Self {
        SendWindow { deadline, open }
    }

    /// When the send must have finished. Past it, fail with `TimedOut`.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// False once the Session has closed. Then fail with `BrokenPipe`.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

/// The USB carrier: SLIP-framed link frames and console text on one byte stream, as the
/// Device's USB-Serial/JTAG port carries them. Any [`Transport`] works, the mock one too.
pub struct UsbCarrier(Box<dyn Transport>);

impl UsbCarrier {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        UsbCarrier(transport)
    }
}

impl Carrier for UsbCarrier {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn FrameReader>, Box<dyn FrameWriter>)> {
        let (reader, writer) = self.0.split()?;
        Ok((
            Box::new(UsbReader {
                reader,
                decoder: StreamDecoder::default(),
                buf: vec![0; 4096],
            }),
            Box::new(UsbWriter(writer)),
        ))
    }
}

struct UsbReader {
    reader: Box<dyn Read + Send>,
    decoder: StreamDecoder,
    buf: Vec<u8>,
}

impl FrameReader for UsbReader {
    fn recv(&mut self) -> io::Result<Received> {
        match self.reader.read(&mut self.buf) {
            Ok(0) => Ok(Received::Ended),
            Ok(n) => Ok(Received::Items(
                self.decoder.feed(&self.buf[..n], Instant::now()),
            )),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                ) =>
            {
                Ok(Received::Items(self.decoder.poll(Instant::now())))
            }
            Err(error) => Err(error),
        }
    }
}

struct UsbWriter(Box<dyn Write + Send>);

impl FrameWriter for UsbWriter {
    fn send(&mut self, out: Outbound<'_>, window: &SendWindow<'_>) -> io::Result<()> {
        let bytes = match out {
            Outbound::Frame(frame) => frame
                .encode()
                .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?,
            Outbound::Line(line) => format!("{line}\n").into_bytes(),
        };
        let mut rest = &bytes[..];
        while !rest.is_empty() {
            if !window.is_open() {
                return Err(ErrorKind::BrokenPipe.into());
            }
            if Instant::now() >= window.deadline() {
                return Err(ErrorKind::TimedOut.into());
            }
            match self.0.write(rest) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => rest = &rest[n..],
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::TimedOut | ErrorKind::Interrupted | ErrorKind::WouldBlock
                    ) =>
                {
                    std::thread::sleep(crate::READ_TICK);
                }
                Err(error) => return Err(error),
            }
        }
        if !window.is_open() {
            return Err(ErrorKind::BrokenPipe.into());
        }
        // Serial drain calls can block indefinitely. Writes enqueue the entire frame;
        // the protocol acknowledgement confirms delivery.
        Ok(())
    }
}

/// The writer a closed Session keeps, so the carrier's own writer drops and frees the link.
pub(crate) struct Released;

impl FrameWriter for Released {
    fn send(&mut self, _out: Outbound<'_>, _window: &SendWindow<'_>) -> io::Result<()> {
        Ok(())
    }
}
