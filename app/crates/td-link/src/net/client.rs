//! The Companion's side of net-v2: pairing by the code the Device shows, and the session
//! carrier that runs link frames in records.

use std::io::{self, ErrorKind, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use td_core::{Frame, Item};
use x25519_dalek::{EphemeralSecret, PublicKey};
use zeroize::Zeroize;

use super::socket::Items;
use super::wire::{
    self, C2D, D2C, PAIR_ACCEPT, PAIR_CHALLENGE, PAIR_COMMIT, PAIR_OFFER, PAIR_PROOF, PAIR_REQUEST,
    PAIR_START, PAIR_STORED, PairKeys, Phase, RECORD_RESERVE, RecordReceiver, RecordSender,
    SESSION_CHALLENGE, SESSION_OPEN, SESSION_PROOF, SESSION_READY, SessionKeys,
};
use super::{
    Address, CompanionId, Credential, DeviceId, NetError, PAIR_ATTEMPT, Refusal, SESSION_SETUP,
    WINDOW_FAILURES, random, random_exponent,
};
use crate::carrier::{Carrier, FrameReader, FrameWriter, Outbound, Received, SendWindow};

/// How long the client looks for the Device closing after an `auth`, to tell a retry from the end.
const CLOSE_CHECK: Duration = Duration::from_millis(150);

/// An open pairing window, bound to this connection. The person reads the code on the Device
/// and [`Pairing::submit`] tries it.
pub struct Pairing {
    items: Items,
    companion_id: CompanionId,
    request: Vec<u8>,
    offer: Vec<u8>,
    device_id: DeviceId,
    window_ends: Instant,
    failures: u8,
    ended: bool,
}

/// A finished pairing. `confirmed` is false when `pair.stored` never arrived: the key stays
/// pending until a session with it succeeds.
#[derive(Debug)]
pub struct Paired {
    pub credential: Credential,
    pub confirmed: bool,
}

/// Asks the Device at `addr` to pair. The Device answers only while a person has opened its
/// pairing window; otherwise this is [`NetError::Refused`] with [`Refusal::Closed`].
pub fn pair(
    addr: &Address,
    companion_id: &CompanionId,
    name: &str,
    timeout: Duration,
) -> Result<Pairing, NetError> {
    if !wire::valid_name(name) {
        return Err(NetError::Protocol("invalid Companion name".into()));
    }
    let deadline = Instant::now() + timeout;
    let mut items = Items::new(addr.connect(timeout)?);
    let mut fields = Vec::with_capacity(17 + name.len());
    fields.extend_from_slice(companion_id);
    fields.push(name.len() as u8);
    fields.extend_from_slice(name.as_bytes());
    let request = wire::handshake(PAIR_REQUEST, &fields)?;
    items.send(&request, deadline)?;
    let (offer, fields) = match items.expect(PAIR_OFFER, deadline) {
        Ok(offer) => offer,
        Err(error) => {
            items.close();
            return Err(error);
        }
    };
    let device_id: DeviceId = fields[..6].try_into().expect("offer length checked");
    let left = u32::from_be_bytes(fields[22..26].try_into().expect("offer length checked"));
    Ok(Pairing {
        items,
        companion_id: *companion_id,
        request,
        offer,
        device_id,
        window_ends: Instant::now() + Duration::from_millis(u64::from(left)),
        failures: 0,
        ended: false,
    })
}

impl Pairing {
    /// The Device id from `pair.offer`. The Companion pins it with the key.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Time left in the Device's pairing window.
    pub fn time_left(&self) -> Duration {
        self.window_ends.saturating_duration_since(Instant::now())
    }

    /// Codes left before the Device closes this window.
    pub fn attempts_left(&self) -> u8 {
        if self.ended {
            0
        } else {
            WINDOW_FAILURES.saturating_sub(self.failures)
        }
    }

    /// Whether another code can still be tried on this connection.
    pub fn is_open(&self) -> bool {
        !self.ended && !self.time_left().is_zero()
    }

    fn end(&mut self) {
        self.ended = true;
        self.items.close();
    }

    /// Tries one code. A wrong code is [`NetError::Refused`] with [`Refusal::Auth`], and the
    /// window stays open for another code while [`Pairing::attempts_left`] is above zero.
    /// `save_pending` stores the key in the credential store before `pair.commit`; if it
    /// fails, pairing stops and the Device discards the key.
    pub fn submit(
        &mut self,
        code: &str,
        save_pending: &mut dyn FnMut(&Credential) -> Result<(), String>,
    ) -> Result<Paired, NetError> {
        if !self.is_open() {
            let expired = !self.ended;
            self.end();
            return Err(if expired {
                NetError::Refused(Refusal::Expired)
            } else {
                NetError::Closed
            });
        }
        if !wire::valid_code(code) {
            return Err(NetError::Protocol("a code is six digits".into()));
        }
        let result = self.attempt(code, save_pending);
        match &result {
            Err(NetError::Refused(Refusal::Auth)) => {
                self.failures += 1;
                if self.failures >= WINDOW_FAILURES || self.items.closed_within(CLOSE_CHECK) {
                    self.end();
                }
            }
            _ => self.end(),
        }
        result
    }

    fn attempt(
        &mut self,
        code: &str,
        save_pending: &mut dyn FnMut(&Credential) -> Result<(), String>,
    ) -> Result<Paired, NetError> {
        let deadline = Instant::now() + PAIR_ATTEMPT.min(self.time_left());
        let mut a = random_exponent();
        let mut client_nonce = [0u8; 32];
        random(&mut client_nonce);
        let a_pub = wire::srp_client_public(&a)?;
        let start = wire::handshake(PAIR_START, &[&a_pub[..], &client_nonce].concat())?;
        self.items.send(&start, deadline)?;
        let (challenge, fields) = self.items.expect(PAIR_CHALLENGE, deadline)?;
        let salt: [u8; 16] = fields[..16].try_into().expect("challenge length checked");
        let b_pub: [u8; 384] = fields[16..400]
            .try_into()
            .expect("challenge length checked");
        let identity = wire::identity(&self.device_id, &self.companion_id);
        let key = wire::srp_client_key(&identity, code, &salt, &a, &b_pub);
        a.zeroize();
        let mut key = key?;
        let keys = PairKeys::derive(&key, &self.request, &self.offer, &start, &challenge);
        key.zeroize();
        let mut keys = keys?;
        // Only the derived keys are needed from here on.
        keys.prk.zeroize();
        self.items
            .send(&wire::handshake(PAIR_PROOF, &keys.client_proof)?, deadline)?;
        let (_, accept) = self.items.expect(PAIR_ACCEPT, deadline)?;
        let credential = Credential {
            device_id: self.device_id,
            companion_id: self.companion_id,
            ltk: keys.accept(&accept)?,
        };
        save_pending(&credential).map_err(NetError::Store)?;
        let commit = wire::handshake(PAIR_COMMIT, &keys.commit_proof(&credential.ltk))?;
        self.items.send(&commit, deadline)?;
        let confirmed = match self.items.expect(PAIR_STORED, deadline) {
            Ok((_, stored)) => wire::same(&stored, &keys.stored_proof(&credential.ltk)),
            Err(_) => false,
        };
        Ok(Paired {
            credential,
            confirmed,
        })
    }

    /// Gives up, which ends the window on the Device.
    pub fn cancel(mut self) {
        self.ended = true;
        self.items.finish();
    }
}

impl Drop for Pairing {
    fn drop(&mut self) {
        self.items.close();
    }
}

/// An authenticated net-v2 session, ready to carry link frames.
pub struct NetCarrier {
    items: Items,
    sender: RecordSender,
    receiver: RecordReceiver,
    device_id: DeviceId,
}

impl NetCarrier {
    /// Connects to `addr` and runs the session exchange with `credential`, within `timeout`
    /// and at most the 10 s net-v2 allows.
    pub fn connect(
        addr: &Address,
        credential: &Credential,
        timeout: Duration,
    ) -> Result<NetCarrier, NetError> {
        let deadline = Instant::now() + timeout.min(SESSION_SETUP);
        let stream = addr.connect(timeout)?;
        Self::open(stream, credential, deadline)
    }

    fn open(
        stream: TcpStream,
        credential: &Credential,
        deadline: Instant,
    ) -> Result<NetCarrier, NetError> {
        let mut items = Items::new(stream);
        let result = Self::handshake(&mut items, credential, deadline);
        match result {
            Ok((sender, receiver)) => Ok(NetCarrier {
                items,
                sender,
                receiver,
                device_id: credential.device_id,
            }),
            Err(error) => {
                items.close();
                Err(error)
            }
        }
    }

    fn handshake(
        items: &mut Items,
        credential: &Credential,
        deadline: Instant,
    ) -> Result<(RecordSender, RecordReceiver), NetError> {
        let secret = EphemeralSecret::random();
        let client_share = PublicKey::from(&secret).to_bytes();
        // No ids on the wire: the tag lets the Device find this Pairing.
        let open = [
            &wire::open_tag(&credential.ltk, &client_share)[..],
            &client_share,
        ]
        .concat();
        items.send(&wire::handshake(SESSION_OPEN, &open)?, deadline)?;
        let (_, fields) = items.expect(SESSION_CHALLENGE, deadline)?;
        let device_share: [u8; 32] = fields[..32].try_into().expect("challenge length checked");
        let shared = wire::agree(secret, &device_share)?;
        let mut keys = SessionKeys::derive(
            &credential.ltk,
            &credential.device_id,
            &credential.companion_id,
            &client_share,
            &device_share,
            shared.as_bytes(),
        )?;
        keys.prk.zeroize();
        drop(shared);
        if !wire::same(&fields[32..], &keys.deck_proof) {
            return Err(NetError::Auth("bad device proof".into()));
        }
        items.send(
            &wire::handshake(SESSION_PROOF, &keys.client_proof)?,
            deadline,
        )?;
        let (_, ready) = items.expect(SESSION_READY, deadline)?;
        if !wire::same(&ready, &keys.ready_proof) {
            return Err(NetError::Auth("bad ready proof".into()));
        }
        Ok((
            RecordSender::new(&keys.c2d, C2D),
            RecordReceiver::new(&keys.d2c, D2C),
        ))
    }

    /// The Device id this session authenticated.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// The next record counter in each direction: sent, then received. A new session starts
    /// both at zero.
    pub fn counters(&self) -> (u64, u64) {
        (self.sender.counter(), self.receiver.counter())
    }

    /// Moves both counters forward to `counter`, for tests of the record limit. The Device's
    /// counters must move the same way, or the next record closes the session.
    #[doc(hidden)]
    pub fn skip_records(&mut self, counter: u64) {
        self.sender.skip_to(counter);
        self.receiver.skip_to(counter);
    }
}

impl Carrier for NetCarrier {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn FrameReader>, Box<dyn FrameWriter>)> {
        let NetCarrier {
            items,
            sender,
            receiver,
            ..
        } = *self;
        let stream = items.stream.try_clone()?;
        let received = Arc::new(AtomicU64::new(receiver.counter()));
        Ok((
            Box::new(NetReader {
                items,
                receiver,
                received: received.clone(),
            }),
            Box::new(NetWriter {
                stream,
                sender,
                received,
            }),
        ))
    }
}

struct NetReader {
    items: Items,
    receiver: RecordReceiver,
    /// The receive counter, for the writer's record limit check.
    received: Arc<AtomicU64>,
}

impl NetReader {
    fn frame(&mut self, item: &[u8]) -> io::Result<Item> {
        let body = self.receiver.open(item).map_err(invalid)?;
        self.received
            .store(self.receiver.counter(), Ordering::Release);
        let frame = Frame::decode_body(&body).map_err(|e| invalid(e.to_string()))?;
        Ok(Item::Frame(frame))
    }
}

fn invalid(error: impl ToString) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, error.to_string())
}

impl FrameReader for NetReader {
    fn recv(&mut self) -> io::Result<Received> {
        let first = match self.items.poll(Phase::Record, crate::READ_TICK) {
            Ok(Some(item)) => item,
            Ok(None) => return Ok(Received::Items(Vec::new())),
            Err(NetError::Closed) => return Ok(Received::Ended),
            Err(error) => {
                self.items.close();
                return Err(invalid(error));
            }
        };
        let mut frames = vec![self.frame(&first)];
        loop {
            match self.items.buffered(Phase::Record) {
                Ok(Some(item)) => frames.push(self.frame(&item)),
                Ok(None) => break,
                Err(error) => frames.push(Err(invalid(error))),
            }
            if frames.last().is_some_and(Result::is_err) {
                break;
            }
        }
        let frames: io::Result<Vec<Item>> = frames.into_iter().collect();
        frames
            .inspect_err(|_| self.items.close())
            .map(Received::Items)
    }
}

impl Drop for NetReader {
    fn drop(&mut self) {
        self.items.close();
    }
}

struct NetWriter {
    stream: TcpStream,
    sender: RecordSender,
    received: Arc<AtomicU64>,
}

impl NetWriter {
    /// Records left in the direction with fewer.
    fn records_left(&self) -> u64 {
        let received = self.received.load(Ordering::Acquire);
        self.sender
            .remaining()
            .min(wire::COUNTER_LIMIT.saturating_sub(received))
    }
}

impl FrameWriter for NetWriter {
    fn start_operation(&mut self) -> io::Result<()> {
        if self.records_left() < RECORD_RESERVE {
            // The keys are nearly used up: end the session between requests. A new session
            // makes fresh keys with counters at zero.
            self.sender.close();
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
            return Err(io::Error::new(
                ErrorKind::ConnectionAborted,
                "the session used up its record limit; reconnect to go on",
            ));
        }
        Ok(())
    }

    fn send(&mut self, out: Outbound<'_>, window: &SendWindow<'_>) -> io::Result<()> {
        let Outbound::Frame(frame) = out else {
            return Err(io::Error::new(
                ErrorKind::Unsupported,
                "a network carrier has no console",
            ));
        };
        if !window.is_open() {
            return Err(ErrorKind::BrokenPipe.into());
        }
        let body = frame.encode_body().map_err(|e| invalid(e.to_string()))?;
        let item = self.sender.seal(&body).map_err(invalid)?;
        let left = window.deadline().saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        self.stream.set_write_timeout(Some(left))?;
        // A record is never retried on the same connection, so a partial write ends it.
        self.stream.write_all(&item).inspect_err(|_| {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        })
    }
}
