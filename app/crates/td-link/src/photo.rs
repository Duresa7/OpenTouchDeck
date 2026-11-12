//! The photo frame, idle mode and clock on a protocol 2 Device, as `schema/protocol/link-v2.md`
//! § "Photo frame, idle, and clock" defines them.
//!
//! `v2.photo.put {bytes, sha256}` opens a transfer for one complete JPEG, the chunks go as for an
//! Asset, and `v2.photo.end {xfer}` shows it. The Device keeps the picture in RAM only. A put while
//! the Device is awake returns `idle`, and one sooner than `interval_s` after the last picture
//! returns `rate` with `retry_ms`. `v2.idle.get` and `v2.idle.set` read and write the idle
//! settings, and `v2.clock.set` sets the clock the Device shows when no picture comes.

use std::fmt;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use td_core::Reply;

use crate::asset::AssetReason;
use crate::session::{LinkError, Session};
use crate::xfer::{XferError, send_chunks};

/// The protocol's ceiling for one picture. A Device advertises its own in `caps.photo_bytes`.
pub const PHOTO_BYTES_MAX: usize = 131_072;
pub const IDLE_TIMEOUT_S: RangeInclusive<u32> = 10..=86_400;
pub const IDLE_INTERVAL_S: RangeInclusive<u32> = 5..=3_600;
pub const CLOCK_UNIX_S_MAX: u64 = 4_102_444_799;
pub const CLOCK_OFFSET_MIN: RangeInclusive<i32> = -840..=840;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const END_TIMEOUT: Duration = Duration::from_secs(5);

/// The Device's idle settings. The fallback is always the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleSettings {
    pub enabled: bool,
    pub timeout_s: u32,
    pub interval_s: u32,
}

impl Default for IdleSettings {
    fn default() -> Self {
        IdleSettings {
            enabled: false,
            timeout_s: 60,
            interval_s: 15,
        }
    }
}

impl IdleSettings {
    pub fn valid(&self) -> bool {
        IDLE_TIMEOUT_S.contains(&self.timeout_s) && IDLE_INTERVAL_S.contains(&self.interval_s)
    }
}

/// What one shown picture took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhotoShown {
    pub bytes: usize,
    pub chunks: u16,
    pub retransmits: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoError {
    Link(LinkError),
    /// The Device refused with this reason. `retry_ms` comes with `rate`.
    Refused {
        reason: AssetReason,
        error: String,
        retry_ms: Option<u64>,
    },
    /// The picture is empty or over the limit, so it never leaves the computer.
    Size {
        bytes: usize,
        limit: usize,
    },
    /// An idle setting or a clock value is out of the protocol's range.
    Invalid(&'static str),
    Xfer(XferError),
}

impl PhotoError {
    pub fn reason(&self) -> Option<&AssetReason> {
        match self {
            PhotoError::Refused { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

impl fmt::Display for PhotoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PhotoError::Link(error) => error.fmt(f),
            PhotoError::Refused {
                reason,
                error,
                retry_ms,
            } => {
                write!(f, "the Device refused: {error} ({})", reason.name())?;
                match retry_ms {
                    Some(ms) => write!(f, ", retry in {ms} ms"),
                    None => Ok(()),
                }
            }
            PhotoError::Size { bytes, limit } => {
                write!(
                    f,
                    "a picture is 1 to {limit} bytes, and this one is {bytes}"
                )
            }
            PhotoError::Invalid(what) => write!(f, "{what} is out of range"),
            PhotoError::Xfer(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PhotoError {}

impl From<LinkError> for PhotoError {
    fn from(error: LinkError) -> Self {
        PhotoError::Link(error)
    }
}

impl From<XferError> for PhotoError {
    fn from(error: XferError) -> Self {
        match error {
            XferError::Link(error) => PhotoError::Link(error),
            other => PhotoError::Xfer(other),
        }
    }
}

fn fields<T: serde::de::DeserializeOwned>(reply: &Reply) -> Result<T, PhotoError> {
    if !reply.ok {
        let text = |key: &str| {
            reply
                .fields
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return Err(PhotoError::Refused {
            reason: AssetReason::parse(&text("reason")),
            error: text("error"),
            retry_ms: reply.fields.get("retry_ms").and_then(Value::as_u64),
        });
    }
    serde_json::from_value(Value::Object(reply.fields.clone())).map_err(|error| {
        PhotoError::Link(LinkError::Reply(td_core::ReplyError::Fields(
            error.to_string(),
        )))
    })
}

fn call<T: serde::de::DeserializeOwned>(
    session: &Session,
    name: &str,
    payload: &impl Serialize,
    timeout: Duration,
) -> Result<T, PhotoError> {
    let bytes = serde_json::to_vec(payload).expect("photo requests serialize");
    fields(&session.call_payload(name, bytes, timeout)?)
}

#[derive(Serialize, Deserialize)]
struct IdleWire {
    enabled: bool,
    timeout_s: u32,
    interval_s: u32,
    fallback: String,
}

impl IdleWire {
    fn settings(self) -> Result<IdleSettings, PhotoError> {
        let settings = IdleSettings {
            enabled: self.enabled,
            timeout_s: self.timeout_s,
            interval_s: self.interval_s,
        };
        if self.fallback != "clock" || !settings.valid() {
            return Err(PhotoError::Link(LinkError::Reply(
                td_core::ReplyError::Fields("the Device sent invalid idle settings".into()),
            )));
        }
        Ok(settings)
    }
}

pub fn idle_get(session: &Session) -> Result<IdleSettings, PhotoError> {
    #[derive(Serialize)]
    struct Get {
        m: &'static str,
    }
    call::<IdleWire>(
        session,
        "v2.idle.get",
        &Get { m: "v2.idle.get" },
        REQUEST_TIMEOUT,
    )?
    .settings()
}

/// Stores the settings on the Device, which answers once they are durable.
pub fn idle_set(session: &Session, settings: IdleSettings) -> Result<IdleSettings, PhotoError> {
    #[derive(Serialize)]
    struct Set {
        m: &'static str,
        enabled: bool,
        timeout_s: u32,
        interval_s: u32,
        fallback: &'static str,
    }
    if !IDLE_TIMEOUT_S.contains(&settings.timeout_s) {
        return Err(PhotoError::Invalid("timeout_s"));
    }
    if !IDLE_INTERVAL_S.contains(&settings.interval_s) {
        return Err(PhotoError::Invalid("interval_s"));
    }
    let request = Set {
        m: "v2.idle.set",
        enabled: settings.enabled,
        timeout_s: settings.timeout_s,
        interval_s: settings.interval_s,
        fallback: "clock",
    };
    call::<IdleWire>(session, request.m, &request, REQUEST_TIMEOUT)?.settings()
}

/// Sets the Device's clock to `unix_s`, shown `offset_min` minutes from UTC.
pub fn clock_set(session: &Session, unix_s: u64, offset_min: i32) -> Result<(), PhotoError> {
    #[derive(Serialize)]
    struct Set {
        m: &'static str,
        unix_s: u64,
        offset_min: i32,
    }
    if unix_s > CLOCK_UNIX_S_MAX {
        return Err(PhotoError::Invalid("unix_s"));
    }
    if !CLOCK_OFFSET_MIN.contains(&offset_min) {
        return Err(PhotoError::Invalid("offset_min"));
    }
    let request = Set {
        m: "v2.clock.set",
        unix_s,
        offset_min,
    };
    call::<Value>(session, request.m, &request, REQUEST_TIMEOUT)?;
    Ok(())
}

/// Sends one JPEG and waits until the Device shows it. `limit` is the Device's `photo_bytes`.
/// A transfer the Device cut, such as one a touch cancelled, reports the reason the Device
/// recorded for it.
pub fn put_photo(session: &Session, data: &[u8], limit: usize) -> Result<PhotoShown, PhotoError> {
    #[derive(Serialize)]
    struct Put<'a> {
        m: &'static str,
        bytes: usize,
        sha256: &'a str,
    }
    #[derive(Serialize)]
    struct End {
        m: &'static str,
        xfer: u8,
    }
    #[derive(Deserialize)]
    struct Opened {
        xfer: u8,
    }
    #[derive(Deserialize)]
    struct Shown {
        shown: bool,
    }
    let limit = limit.min(PHOTO_BYTES_MAX);
    if data.is_empty() || data.len() > limit {
        return Err(PhotoError::Size {
            bytes: data.len(),
            limit,
        });
    }
    let started = Instant::now();
    let sha256 = crate::bundle::sha256(data);
    let mut transfer = session.transfer()?;
    let put = Put {
        m: "v2.photo.put",
        bytes: data.len(),
        sha256: &sha256,
    };
    let opened: Opened = fields(&transfer.begin(|| {
        session.call_payload(
            put.m,
            serde_json::to_vec(&put).expect("photo request serializes"),
            REQUEST_TIMEOUT,
        )
    })?)?;
    let end = End {
        m: "v2.photo.end",
        xfer: opened.xfer,
    };
    let (chunks, retransmits) = match send_chunks(&transfer, opened.xfer, data) {
        Ok(sent) => sent,
        Err(error) => {
            return Err(
                match transfer.finish(|| call::<Shown>(session, end.m, &end, REQUEST_TIMEOUT)) {
                    Err(refused @ PhotoError::Refused { .. }) => refused,
                    _ => error.into(),
                },
            );
        }
    };
    let shown: Shown = fields(&transfer.finish(|| {
        session.call_payload(
            end.m,
            serde_json::to_vec(&end).expect("photo request serializes"),
            END_TIMEOUT,
        )
    })?)?;
    if !shown.shown {
        return Err(PhotoError::Link(LinkError::Reply(
            td_core::ReplyError::Fields("v2.photo.end did not show the picture".into()),
        )));
    }
    Ok(PhotoShown {
        bytes: data.len(),
        chunks,
        retransmits,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
