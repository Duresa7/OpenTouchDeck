//! Assets on a protocol 2 Device, as `schema/protocol/link-v2.md` § "Assets and bulk transfers" defines them.
//!
//! An Asset's id is the SHA-256 of its exact bytes, 64 lowercase hex digits. `v2.asset.stat` reports the
//! space, `v2.asset.list` pages through the stored Assets four at a time, `v2.asset.put {id, bytes, format}`
//! either answers `present:true` for an Asset the Device already has or opens a transfer, the chunks go as
//! for a Layout, and `v2.asset.end {xfer}` commits it. `v2.asset.delete {id}` removes one. Every call here
//! needs protocol 2 selected first, which [`Session::identity`] does.

use std::collections::BTreeSet;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use td_core::Reply;

use crate::session::{LinkError, Session};
use crate::xfer::{ListingBudget, XferError, listing_error, send_chunks};

/// The most Assets one `v2.asset.list` reply holds.
pub const ASSET_PAGE: usize = 4;
/// The protocol's ceiling for one Asset. A Device advertises its own, lower one in `caps.asset_bytes`.
pub const ASSET_BYTES_MAX: usize = 1_048_576;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const END_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetFormat {
    Tdi1,
    Gif,
    Jpeg,
}

impl AssetFormat {
    pub fn name(self) -> &'static str {
        match self {
            AssetFormat::Tdi1 => "tdi1",
            AssetFormat::Gif => "gif",
            AssetFormat::Jpeg => "jpeg",
        }
    }

    pub fn parse(name: &str) -> Option<AssetFormat> {
        match name {
            "tdi1" => Some(AssetFormat::Tdi1),
            "gif" => Some(AssetFormat::Gif),
            "jpeg" => Some(AssetFormat::Jpeg),
            _ => None,
        }
    }

    /// The format the bytes start with, or `None`. The Device still validates the whole file.
    pub fn sniff(data: &[u8]) -> Option<AssetFormat> {
        if data.starts_with(b"TDI1") {
            Some(AssetFormat::Tdi1)
        } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
            Some(AssetFormat::Gif)
        } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(AssetFormat::Jpeg)
        } else {
            None
        }
    }
}

impl fmt::Display for AssetFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The id of these bytes: their SHA-256 in lowercase hex.
pub fn asset_id(data: &[u8]) -> String {
    crate::bundle::sha256(data)
}

/// Whether `id` is 64 lowercase hex digits.
pub fn valid_asset_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The Device's space for Assets. `free_bytes` is what is left after `used_bytes` and the
/// `reserved_bytes` for open uploads and filesystem headroom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetSpace {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetItem {
    pub id: String,
    pub bytes: u64,
    pub format: AssetFormat,
}

/// One `v2.asset.list` reply: the Assets after the cursor, and the cursor for the next page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetPage {
    pub items: Vec<AssetItem>,
    pub next: Option<String>,
}

/// What `v2.asset.put` answers: the Asset is already there, or a transfer is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutOpened {
    pub present: bool,
    pub xfer: Option<u8>,
}

/// What `v2.asset.end` answers after a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetCommitted {
    pub id: String,
    pub bytes: u64,
}

/// What one [`put_asset`] did. `sent` is false when the Device already had the Asset, and then no
/// chunk went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssetPut {
    pub id: String,
    pub bytes: usize,
    pub format: AssetFormat,
    pub sent: bool,
    pub chunks: u16,
    pub retransmits: u32,
    pub elapsed_ms: u64,
}

/// The stable `reason` of a refused v2 request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssetReason {
    NotNegotiated,
    Args,
    Unsupported,
    Memory,
    Storage,
    Busy,
    Size,
    Full,
    Format,
    State,
    Incomplete,
    Hash,
    InUse,
    Idle,
    Rate,
    Other(String),
}

impl AssetReason {
    pub fn parse(reason: &str) -> AssetReason {
        match reason {
            "not-negotiated" => AssetReason::NotNegotiated,
            "args" => AssetReason::Args,
            "unsupported" => AssetReason::Unsupported,
            "memory" => AssetReason::Memory,
            "storage" => AssetReason::Storage,
            "busy" => AssetReason::Busy,
            "size" => AssetReason::Size,
            "full" => AssetReason::Full,
            "format" => AssetReason::Format,
            "state" => AssetReason::State,
            "incomplete" => AssetReason::Incomplete,
            "hash" => AssetReason::Hash,
            "in-use" => AssetReason::InUse,
            "idle" => AssetReason::Idle,
            "rate" => AssetReason::Rate,
            other => AssetReason::Other(other.to_owned()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            AssetReason::NotNegotiated => "not-negotiated",
            AssetReason::Args => "args",
            AssetReason::Unsupported => "unsupported",
            AssetReason::Memory => "memory",
            AssetReason::Storage => "storage",
            AssetReason::Busy => "busy",
            AssetReason::Size => "size",
            AssetReason::Full => "full",
            AssetReason::Format => "format",
            AssetReason::State => "state",
            AssetReason::Incomplete => "incomplete",
            AssetReason::Hash => "hash",
            AssetReason::InUse => "in-use",
            AssetReason::Idle => "idle",
            AssetReason::Rate => "rate",
            AssetReason::Other(other) => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    Link(LinkError),
    /// The Device refused the request with this reason and message.
    Refused {
        reason: AssetReason,
        error: String,
    },
    /// The file is empty or over the protocol's 1 MiB ceiling, so it never leaves the computer.
    Size(usize),
    /// The id is not 64 lowercase hex digits.
    Id(String),
    Xfer(XferError),
}

impl AssetError {
    pub fn reason(&self) -> Option<&AssetReason> {
        match self {
            AssetError::Refused { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Link(error) => error.fmt(f),
            AssetError::Refused {
                reason: AssetReason::Full,
                ..
            } => f.write_str(
                "the Device has no room for this Asset (full); remove unused Assets and try again",
            ),
            AssetError::Refused {
                reason: AssetReason::NotNegotiated,
                ..
            } => f.write_str("the Device needs protocol 2 for Assets (not-negotiated)"),
            AssetError::Refused { reason, error } => {
                write!(
                    f,
                    "the Device refused the Asset: {error} ({})",
                    reason.name()
                )
            }
            AssetError::Size(bytes) => write!(
                f,
                "an Asset is 1 to {ASSET_BYTES_MAX} bytes, and this one is {bytes}"
            ),
            AssetError::Id(id) => write!(f, "{id:?} is not an Asset id of 64 lowercase hex digits"),
            AssetError::Xfer(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AssetError {}

impl From<LinkError> for AssetError {
    fn from(error: LinkError) -> Self {
        AssetError::Link(error)
    }
}

impl From<XferError> for AssetError {
    fn from(error: XferError) -> Self {
        match error {
            XferError::Link(error) => AssetError::Link(error),
            other => AssetError::Xfer(other),
        }
    }
}

/// The v2 Asset requests. Each serializes with `m` first and its fields in the protocol's order,
/// byte for byte as `schema/protocol/golden-frames-v2.json` shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRequest<'a> {
    Stat,
    List {
        after: Option<&'a str>,
    },
    Put {
        id: &'a str,
        bytes: usize,
        format: AssetFormat,
    },
    End {
        xfer: u8,
    },
    Delete {
        id: &'a str,
    },
}

impl AssetRequest<'_> {
    pub fn name(&self) -> &'static str {
        match self {
            AssetRequest::Stat => "v2.asset.stat",
            AssetRequest::List { .. } => "v2.asset.list",
            AssetRequest::Put { .. } => "v2.asset.put",
            AssetRequest::End { .. } => "v2.asset.end",
            AssetRequest::Delete { .. } => "v2.asset.delete",
        }
    }

    pub fn to_payload(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Stat {
            m: &'static str,
        }
        #[derive(Serialize)]
        struct List<'a> {
            m: &'static str,
            after: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct Put<'a> {
            m: &'static str,
            id: &'a str,
            bytes: usize,
            format: AssetFormat,
        }
        #[derive(Serialize)]
        struct End {
            m: &'static str,
            xfer: u8,
        }
        #[derive(Serialize)]
        struct Delete<'a> {
            m: &'static str,
            id: &'a str,
        }
        let m = self.name();
        match *self {
            AssetRequest::Stat => serde_json::to_vec(&Stat { m }),
            AssetRequest::List { after } => serde_json::to_vec(&List { m, after }),
            AssetRequest::Put { id, bytes, format } => serde_json::to_vec(&Put {
                m,
                id,
                bytes,
                format,
            }),
            AssetRequest::End { xfer } => serde_json::to_vec(&End { m, xfer }),
            AssetRequest::Delete { id } => serde_json::to_vec(&Delete { m, id }),
        }
        .expect("Asset requests serialize")
    }
}

/// The fields of a v2 reply, or the typed refusal it carries.
pub fn reply_fields<T: serde::de::DeserializeOwned>(reply: &Reply) -> Result<T, AssetError> {
    if !reply.ok {
        let text = |key: &str| {
            reply
                .fields
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return Err(AssetError::Refused {
            reason: AssetReason::parse(&text("reason")),
            error: text("error"),
        });
    }
    serde_json::from_value(Value::Object(reply.fields.clone())).map_err(|error| {
        AssetError::Link(LinkError::Reply(td_core::ReplyError::Fields(
            error.to_string(),
        )))
    })
}

fn call<T: serde::de::DeserializeOwned>(
    session: &Session,
    request: &AssetRequest,
    timeout: Duration,
) -> Result<T, AssetError> {
    let reply = session.call_payload(request.name(), request.to_payload(), timeout)?;
    reply_fields(&reply)
}

pub fn asset_space(session: &Session) -> Result<AssetSpace, AssetError> {
    call(session, &AssetRequest::Stat, REQUEST_TIMEOUT)
}

pub fn list_asset_page(session: &Session, after: Option<&str>) -> Result<AssetPage, AssetError> {
    call(session, &AssetRequest::List { after }, REQUEST_TIMEOUT)
}

/// Every Asset on the Device, a page at a time.
pub fn list_assets(session: &Session) -> Result<Vec<AssetItem>, AssetError> {
    let mut items: Vec<AssetItem> = Vec::new();
    let mut after: Option<String> = None;
    let mut cursors = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut budget = ListingBudget::new();
    loop {
        let page: AssetPage = call(
            session,
            &AssetRequest::List {
                after: after.as_deref(),
            },
            budget.timeout()?,
        )?;
        budget.page(page.items.len())?;
        for item in &page.items {
            if !ids.insert(item.id.clone()) {
                return Err(listing_error("repeated an item").into());
            }
        }
        items.extend(page.items);
        match page.next {
            Some(next) => {
                if !cursors.insert(next.clone()) {
                    return Err(listing_error("repeated a cursor").into());
                }
                after = Some(next);
            }
            None => return Ok(items),
        }
    }
}

/// Stores `data` as an Asset unless the Device already has it. A put of an Asset the Device holds
/// sends no chunk. A put after a lost connection is safe to repeat: the id names the bytes.
pub fn put_asset(
    session: &Session,
    data: &[u8],
    format: AssetFormat,
) -> Result<AssetPut, AssetError> {
    if data.is_empty() || data.len() > ASSET_BYTES_MAX {
        return Err(AssetError::Size(data.len()));
    }
    let started = Instant::now();
    let id = asset_id(data);
    let mut transfer = session.transfer()?;
    let request = AssetRequest::Put {
        id: &id,
        bytes: data.len(),
        format,
    };
    let opened: PutOpened =
        reply_fields(&transfer.begin(|| {
            session.call_payload(request.name(), request.to_payload(), REQUEST_TIMEOUT)
        })?)?;
    let mut report = AssetPut {
        id: id.clone(),
        bytes: data.len(),
        format,
        sent: false,
        chunks: 0,
        retransmits: 0,
        elapsed_ms: 0,
    };
    if !opened.present {
        let xfer = opened.xfer.ok_or_else(|| {
            AssetError::Link(LinkError::Reply(td_core::ReplyError::Fields(
                "v2.asset.put opened no xfer".into(),
            )))
        })?;
        let (chunks, retransmits) = send_chunks(&transfer, xfer, data)?;
        let end = AssetRequest::End { xfer };
        let committed: AssetCommitted = reply_fields(
            &transfer.finish(|| session.call_payload(end.name(), end.to_payload(), END_TIMEOUT))?,
        )?;
        if committed.id != id || committed.bytes != data.len() as u64 {
            return Err(AssetError::Link(LinkError::Reply(
                td_core::ReplyError::Fields(format!(
                    "the Device committed {} bytes as {}, not {} bytes as {id}",
                    committed.bytes,
                    committed.id,
                    data.len()
                )),
            )));
        }
        report.sent = true;
        report.chunks = chunks;
        report.retransmits = retransmits;
    }
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

/// Removes one Asset. Returns false when the Device did not have it.
pub fn delete_asset(session: &Session, id: &str) -> Result<bool, AssetError> {
    if !valid_asset_id(id) {
        return Err(AssetError::Id(id.to_owned()));
    }
    #[derive(Deserialize)]
    struct Deleted {
        deleted: bool,
    }
    let deleted: Deleted = call(session, &AssetRequest::Delete { id }, REQUEST_TIMEOUT)?;
    Ok(deleted.deleted)
}

/// The reply fields a refusal carries, for a mock or a test that builds one.
pub fn refusal(reason: &str, error: &str) -> Map<String, Value> {
    Map::from_iter([
        ("ok".to_owned(), Value::Bool(false)),
        ("reason".to_owned(), Value::String(reason.to_owned())),
        ("error".to_owned(), Value::String(error.to_owned())),
    ])
}
