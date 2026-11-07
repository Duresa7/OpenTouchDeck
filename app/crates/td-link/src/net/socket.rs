//! Items on a TCP stream: `u16be(len) || body`, read whole, with the length checked against the
//! current phase's bounds before anything is kept.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

use super::wire::{self, ERROR, Phase};
use super::{NetError, Refusal};

/// How long a terminal error waits for the peer before the socket closes.
pub(crate) const DRAIN: Duration = Duration::from_millis(100);

pub(crate) struct Items {
    pub(crate) stream: TcpStream,
    buf: Vec<u8>,
}

fn timed_out() -> NetError {
    NetError::Io(io::Error::from(ErrorKind::TimedOut))
}

impl Items {
    pub(crate) fn new(stream: TcpStream) -> Self {
        let _ = stream.set_nodelay(true);
        Items {
            stream,
            buf: Vec::with_capacity(1_100),
        }
    }

    /// Writes one whole item, or fails by `deadline`.
    pub(crate) fn send(&mut self, item: &[u8], deadline: Instant) -> Result<(), NetError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(timed_out());
        }
        self.stream.set_write_timeout(Some(left))?;
        self.stream.write_all(item)?;
        Ok(())
    }

    /// The next whole item, prefix included, if one is buffered. A length outside the
    /// phase's bounds is a protocol error.
    fn take(&mut self, phase: Phase) -> Result<Option<Vec<u8>>, NetError> {
        if self.buf.len() < 2 {
            return Ok(None);
        }
        let len = phase.body_len([self.buf[0], self.buf[1]])?;
        if self.buf.len() < len + 2 {
            return Ok(None);
        }
        Ok(Some(self.buf.drain(..len + 2).collect()))
    }

    /// One read that waits at most `wait`. `Ok(false)` means nothing arrived in time.
    fn fill(&mut self, wait: Duration) -> Result<bool, NetError> {
        self.stream
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        let mut chunk = [0u8; 2_048];
        match self.stream.read(&mut chunk) {
            Ok(0) if self.buf.is_empty() => Err(NetError::Closed),
            Ok(0) => Err(NetError::Protocol("the stream ended inside an item".into())),
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(true)
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Reads one whole item by `deadline`. Partial reads never extend it.
    pub(crate) fn read(&mut self, phase: Phase, deadline: Instant) -> Result<Vec<u8>, NetError> {
        loop {
            if let Some(item) = self.take(phase)? {
                return Ok(item);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(timed_out());
            }
            self.fill(left)?;
        }
    }

    /// The next whole item already read, without waiting.
    pub(crate) fn buffered(&mut self, phase: Phase) -> Result<Option<Vec<u8>>, NetError> {
        self.take(phase)
    }

    /// One read of at most `wait`, then the next whole item if there is one.
    pub(crate) fn poll(
        &mut self,
        phase: Phase,
        wait: Duration,
    ) -> Result<Option<Vec<u8>>, NetError> {
        if let Some(item) = self.take(phase)? {
            return Ok(Some(item));
        }
        self.fill(wait)?;
        self.take(phase)
    }

    /// Reads one handshake item of `kind` by `deadline`. An error item is [`NetError::Refused`].
    pub(crate) fn expect(
        &mut self,
        kind: u8,
        deadline: Instant,
    ) -> Result<(Vec<u8>, Vec<u8>), NetError> {
        let item = self.read(Phase::Handshake, deadline)?;
        let (got, fields) = wire::parse_handshake(&item)?;
        if got == ERROR {
            let reason = Refusal::from_u8(fields[0]).expect("parse_handshake checked the reason");
            return Err(NetError::Refused(reason));
        }
        if got != kind {
            return Err(NetError::Protocol(format!(
                "expected item 0x{kind:02x}, got 0x{got:02x}"
            )));
        }
        let fields = fields.to_vec();
        Ok((item, fields))
    }

    /// Whether the peer closed within `wait`, with nothing more to read.
    pub(crate) fn closed_within(&mut self, wait: Duration) -> bool {
        matches!(self.fill(wait), Err(NetError::Closed))
    }

    /// After a terminal error: shut down the write side, drain briefly, then close.
    pub(crate) fn finish(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Write);
        let end = Instant::now() + DRAIN;
        let mut scratch = [0u8; 512];
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let _ = self
                .stream
                .set_read_timeout(Some(left.max(Duration::from_millis(1))));
            match self.stream.read(&mut scratch) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        self.close();
    }

    pub(crate) fn close(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}
