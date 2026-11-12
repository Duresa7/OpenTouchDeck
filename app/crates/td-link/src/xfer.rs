//! Layout transfers over the link, in M2-05's shapes and Appendix G item G1's limits.
//!
//! `layout.put {orientation, bytes, crc32}` and `layout.get {orientation}` each open a
//! transfer, and the reply names it with `xfer`. The sender writes chunks of up to 1021 bytes,
//! at most 4 ahead of the last ack. Each chunk is a request frame with a fresh `seq`, and the
//! receiver answers it with an ack frame that echoes that `seq` and names the next index it
//! wants. A chunk without an ack after 200 ms goes again, 5 times in all, and then the
//! transfer aborts. `xfer.end {xfer}` closes it: after a put the deck checks the CRC-32,
//! swaps the file in, and replies with its `layout state` fields. After a get the app checks
//! the CRC-32 itself. `icon.put {id, bytes, crc32}` opens a transfer the same way for one
//! 12300-byte icon file, and `icon.rm {id}` removes one.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::time::{Duration, Instant};

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use td_core::frame::CHUNK_DATA_MAX;
use td_core::layout::UPLOAD_MAX;
use td_core::{IconPage, Layout, Orientation, ReplyError, Request};

use crate::session::{LinkError, Session, Transfer, XferFrame};

pub const WINDOW: u16 = 4;
pub const RETRY_AFTER: Duration = Duration::from_millis(200);
pub const TRANSMISSIONS: u32 = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const END_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a receiver waits for the next chunk before it gives up on the sender.
const IDLE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) const LIST_ITEMS_MAX: usize = 4096;
const LIST_PAGES_MAX: usize = 4096;
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct ListingBudget {
    deadline: Instant,
    pages: usize,
    items: usize,
}

pub(crate) fn listing_error(reason: &str) -> LinkError {
    LinkError::Reply(ReplyError::Fields(format!("Device listing {reason}")))
}

impl ListingBudget {
    pub(crate) fn new() -> Self {
        Self {
            deadline: Instant::now() + LIST_TIMEOUT,
            pages: 0,
            items: 0,
        }
    }

    pub(crate) fn timeout(&self) -> Result<Duration, LinkError> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(LinkError::Timeout(LIST_TIMEOUT));
        }
        Ok(left.min(REQUEST_TIMEOUT))
    }

    pub(crate) fn page(&mut self, items: usize) -> Result<(), LinkError> {
        self.timeout()?;
        self.pages += 1;
        self.items += items;
        if self.pages > LIST_PAGES_MAX || self.items > LIST_ITEMS_MAX {
            return Err(listing_error("exceeds page or item limit"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XferError {
    Link(LinkError),
    TooLarge(usize),
    Orientation {
        declared: Orientation,
        target: Orientation,
    },
    Aborted {
        idx: u16,
    },
    Stalled {
        received: usize,
        bytes: usize,
    },
    Crc {
        expected: u32,
        got: u32,
    },
    Size {
        expected: usize,
        got: usize,
    },
}

impl fmt::Display for XferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XferError::Link(error) => error.fmt(f),
            XferError::TooLarge(bytes) => write!(
                f,
                "layout is {bytes} bytes; the deck takes at most {UPLOAD_MAX}"
            ),
            XferError::Orientation { declared, target } => write!(
                f,
                "layout declares {} orientation, but the target slot is {}",
                orientation_name(*declared),
                orientation_name(*target)
            ),
            XferError::Aborted { idx } => write!(
                f,
                "chunk {idx} went {TRANSMISSIONS} times without an ack; transfer aborted"
            ),
            XferError::Stalled { received, bytes } => write!(
                f,
                "the deck stopped sending after {received} of {bytes} bytes"
            ),
            XferError::Crc { expected, got } => {
                write!(f, "CRC-32 is {got:08x}, the deck said {expected:08x}")
            }
            XferError::Size { expected, got } => {
                write!(f, "received {got} bytes, the deck said {expected}")
            }
        }
    }
}

impl std::error::Error for XferError {}

impl From<LinkError> for XferError {
    fn from(error: LinkError) -> Self {
        XferError::Link(error)
    }
}

impl From<ReplyError> for XferError {
    fn from(error: ReplyError) -> Self {
        XferError::Link(LinkError::Reply(error))
    }
}

/// The deck's `layout state` fields, which `xfer.end` returns after a put.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutState {
    pub source: String,
    pub name: String,
    pub orientation: Orientation,
    pub pages: u32,
    pub tiles: u32,
    #[serde(default)]
    pub icons_loaded: Option<u32>,
    #[serde(default)]
    pub icons_missing: Option<u32>,
    #[serde(default)]
    pub bytes: Option<u32>,
    #[serde(default)]
    pub load_ms: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PutReport {
    pub state: LayoutState,
    pub bytes: usize,
    pub chunks: u16,
    pub retransmits: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GetReport {
    #[serde(skip)]
    pub data: Vec<u8>,
    pub bytes: usize,
    pub crc32: u32,
    pub elapsed_ms: u64,
}

#[derive(Deserialize)]
struct Opened {
    xfer: u8,
    #[serde(default)]
    bytes: Option<usize>,
    #[serde(default)]
    crc32: Option<u32>,
}

/// CRC-32 as zlib and Ethernet compute it: polynomial `0x04C11DB7` reflected, initial value
/// and final XOR `0xFFFFFFFF`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn orientation_name(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Landscape => "landscape",
        Orientation::Portrait => "portrait",
    }
}

fn chunk_count(bytes: usize) -> u16 {
    u16::try_from(bytes.div_ceil(CHUNK_DATA_MAX)).expect("an upload fits 65535 chunks")
}

/// Every host writer keeps the declared orientation equal to the target storage slot.
pub fn check_layout_orientation(layout: &Layout, target: Orientation) -> Result<(), XferError> {
    if layout.orientation != target {
        return Err(XferError::Orientation {
            declared: layout.orientation,
            target,
        });
    }
    Ok(())
}

/// Pushes a layout into the deck's slot for `orientation` in its canonical compact form.
pub fn put_layout(
    session: &Session,
    orientation: Orientation,
    layout: &Layout,
) -> Result<PutReport, XferError> {
    put_bytes(session, orientation, &layout.to_canonical_json())
}

/// Pushes raw layout bytes. The app sends only canonical layouts through [`put_layout`];
/// this path exists for tests and the CLI.
pub fn put_bytes(
    session: &Session,
    orientation: Orientation,
    data: &[u8],
) -> Result<PutReport, XferError> {
    if data.len() > UPLOAD_MAX {
        return Err(XferError::TooLarge(data.len()));
    }
    if let Ok(layout) = Layout::parse(data) {
        check_layout_orientation(&layout, orientation)?;
    }
    let started = Instant::now();
    let mut transfer = session.transfer()?;
    let request = json!({
        "m": "layout.put",
        "orientation": orientation_name(orientation),
        "bytes": data.len(),
        "crc32": crc32(data),
    });
    let opened: Opened = transfer
        .begin(|| session.call(&request, REQUEST_TIMEOUT))?
        .fields()?;
    let (chunks, retransmits) = send_chunks(&transfer, opened.xfer, data)?;
    let state: LayoutState = transfer
        .finish(|| session.call(&json!({"m": "xfer.end", "xfer": opened.xfer}), END_TIMEOUT))?
        .fields()?;
    Ok(PutReport {
        state,
        bytes: data.len(),
        chunks,
        retransmits,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

pub(crate) fn send_chunks(
    transfer: &Transfer,
    xfer: u8,
    data: &[u8],
) -> Result<(u16, u32), XferError> {
    let deadline = transfer.data_deadline(data.len());
    let mut progress = Instant::now();
    let count = chunk_count(data.len());
    let chunk = |idx: u16| {
        let start = usize::from(idx) * CHUNK_DATA_MAX;
        &data[start..(start + CHUNK_DATA_MAX).min(data.len())]
    };
    let (mut base, mut ahead) = (0u16, 0u16);
    let mut sent: HashMap<u16, (Instant, u32)> = HashMap::new();
    let mut retransmits = 0;
    while base < count {
        if Instant::now() >= deadline || progress.elapsed() >= IDLE_TIMEOUT {
            return Err(XferError::Stalled {
                received: usize::from(base) * CHUNK_DATA_MAX,
                bytes: data.len(),
            });
        }
        while ahead < count && ahead < base + WINDOW {
            transfer.send_chunk(xfer, ahead, chunk(ahead))?;
            sent.insert(ahead, (Instant::now(), 1));
            ahead += 1;
        }
        let due = (base..ahead)
            .filter_map(|idx| sent.get(&idx).map(|(at, _)| *at + RETRY_AFTER))
            .min()
            .expect("an unacked chunk is in flight");
        let wait = due
            .min(deadline)
            .min(progress + IDLE_TIMEOUT)
            .saturating_duration_since(Instant::now());
        match transfer.frames.recv_timeout(wait) {
            Ok(XferFrame::Ack { xfer: id, next }) if id == xfer => {
                if next > base && next <= ahead {
                    base = next.min(ahead);
                    sent.retain(|idx, _| *idx >= base);
                    progress = Instant::now();
                }
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(XferError::Link(LinkError::Closed(
                    "the transfer channel closed".into(),
                )));
            }
        }
        let now = Instant::now();
        for idx in base..ahead {
            let Some((at, times)) = sent.get_mut(&idx) else {
                continue;
            };
            if *at + RETRY_AFTER > now {
                continue;
            }
            if *times >= TRANSMISSIONS {
                return Err(XferError::Aborted { idx });
            }
            transfer.send_chunk(xfer, idx, chunk(idx))?;
            *at = now;
            *times += 1;
            retransmits += 1;
        }
    }
    Ok((count, retransmits))
}

/// Pulls the layout the deck stores for `orientation` and checks its size and CRC-32.
pub fn get_layout(session: &Session, orientation: Orientation) -> Result<GetReport, XferError> {
    let started = Instant::now();
    let mut transfer = session.transfer()?;
    let opened: Opened = transfer
        .begin(|| {
            session.call(
                &json!({"m": "layout.get", "orientation": orientation_name(orientation)}),
                REQUEST_TIMEOUT,
            )
        })?
        .fields()?;
    let bytes = opened.bytes.ok_or_else(|| {
        XferError::from(ReplyError::Fields("layout.get reply has no bytes".into()))
    })?;
    let expected_crc = opened.crc32.ok_or_else(|| {
        XferError::from(ReplyError::Fields("layout.get reply has no crc32".into()))
    })?;
    if bytes > UPLOAD_MAX {
        return Err(XferError::TooLarge(bytes));
    }
    let count = chunk_count(bytes);
    let mut data = Vec::with_capacity(bytes);
    let mut next = 0u16;
    let deadline = transfer.data_deadline(bytes);
    let mut progress = Instant::now();
    while next < count {
        let until = deadline.min(progress + IDLE_TIMEOUT);
        if Instant::now() >= until {
            return Err(XferError::Stalled {
                received: data.len(),
                bytes,
            });
        }
        let frame = transfer
            .frames
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .map_err(|_| XferError::Stalled {
                received: data.len(),
                bytes,
            })?;
        let XferFrame::Chunk {
            seq,
            xfer,
            idx,
            data: chunk,
        } = frame
        else {
            continue;
        };
        if xfer != opened.xfer {
            continue;
        }
        if idx == next && !chunk.is_empty() {
            if data.len() + chunk.len() > bytes {
                return Err(XferError::Size {
                    expected: bytes,
                    got: data.len() + chunk.len(),
                });
            }
            data.extend_from_slice(&chunk);
            next += 1;
            progress = Instant::now();
        }
        transfer.send_ack(seq, xfer, next)?;
    }
    transfer
        .finish(|| session.call(&json!({"m": "xfer.end", "xfer": opened.xfer}), END_TIMEOUT))?
        .fields::<Value>()?;
    if data.len() != bytes {
        return Err(XferError::Size {
            expected: bytes,
            got: data.len(),
        });
    }
    let got = crc32(&data);
    if got != expected_crc {
        return Err(XferError::Crc {
            expected: expected_crc,
            got,
        });
    }
    Ok(GetReport {
        data,
        bytes,
        crc32: got,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// A `TDI1` icon file is always this size, per `schema/icon-format.md`.
pub const ICON_BYTES: usize = 12300;

/// What one icon sync did before a layout push.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IconSync {
    /// Ids the layout names that the deck lacked and the app sent.
    pub sent: Vec<String>,
    /// Ids the layout names that the deck lacked and the app has no file for.
    pub unavailable: Vec<String>,
}

/// Every icon id the deck stores, read a page at a time.
pub fn list_icons(session: &Session) -> Result<BTreeSet<String>, XferError> {
    let mut ids = BTreeSet::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut budget = ListingBudget::new();
    loop {
        let page: IconPage = session
            .request(&Request::IconList { cursor }, budget.timeout()?)?
            .fields()?;
        budget.page(page.ids.len())?;
        for id in page.ids {
            if !ids.insert(id) {
                return Err(listing_error("repeated an item").into());
            }
        }
        match page.next {
            Some(next) => {
                if !seen.insert(next) {
                    return Err(listing_error("repeated a cursor").into());
                }
                cursor = Some(next);
            }
            None => return Ok(ids),
        }
    }
}

/// Stores one icon file on the deck as `<id>.bin`.
pub fn put_icon(session: &Session, id: &str, data: &[u8]) -> Result<(), XferError> {
    if data.len() != ICON_BYTES {
        return Err(XferError::Size {
            expected: ICON_BYTES,
            got: data.len(),
        });
    }
    let mut transfer = session.transfer()?;
    let request = json!({"m": "icon.put", "id": id, "bytes": data.len(), "crc32": crc32(data)});
    let opened: Opened = transfer
        .begin(|| session.call(&request, REQUEST_TIMEOUT))?
        .fields()?;
    send_chunks(&transfer, opened.xfer, data)?;
    transfer
        .finish(|| session.call(&json!({"m": "xfer.end", "xfer": opened.xfer}), END_TIMEOUT))?
        .fields::<IgnoredAny>()?;
    Ok(())
}

pub fn remove_icon(session: &Session, id: &str) -> Result<(), XferError> {
    session
        .call(&json!({"m": "icon.rm", "id": id}), REQUEST_TIMEOUT)?
        .fields::<IgnoredAny>()?;
    Ok(())
}

/// Sends every id in `wanted` that the deck lacks, reading each file through `file`.
pub fn push_icons(
    session: &Session,
    wanted: &BTreeSet<String>,
    file: impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<IconSync, XferError> {
    let stored = list_icons(session)?;
    let mut sync = IconSync::default();
    for id in wanted.difference(&stored) {
        match file(id) {
            Some(data) => {
                put_icon(session, id, &data)?;
                sync.sent.push(id.clone());
            }
            None => sync.unavailable.push(id.clone()),
        }
    }
    Ok(sync)
}

/// Removes every icon on the deck that is not in `keep`, and returns the removed ids.
pub fn prune_icons(session: &Session, keep: &BTreeSet<String>) -> Result<Vec<String>, XferError> {
    let unused: Vec<String> = list_icons(session)?.difference(keep).cloned().collect();
    for id in &unused {
        remove_icon(session, id)?;
    }
    Ok(unused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MockDeck, SessionConfig};

    #[test]
    fn listing_budget_bounds_empty_pages_items_and_elapsed_time() {
        let mut pages = ListingBudget::new();
        for _ in 0..LIST_PAGES_MAX {
            pages.page(0).unwrap();
        }
        assert!(pages.page(0).is_err());
        let mut items = ListingBudget::new();
        items.page(LIST_ITEMS_MAX).unwrap();
        assert!(items.page(1).is_err());
        let mut expired = ListingBudget::new();
        expired.deadline = Instant::now();
        assert!(expired.timeout().is_err());
        assert!(expired.page(0).is_err());
    }

    fn dev() -> Layout {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../schema/examples/dev.json"),
        )
        .unwrap();
        Layout::parse(&bytes).unwrap()
    }

    fn open(deck: &MockDeck) -> Session {
        Session::open(
            Box::new(deck.connect()),
            SessionConfig {
                ping_every: None,
                ..SessionConfig::default()
            },
        )
        .unwrap()
        .0
    }

    fn counts(layout: &Layout) -> (u32, u32) {
        let tiles = layout
            .pages
            .iter()
            .map(|page| match page {
                td_core::layout::Page::Grid { tiles, .. } => tiles.len() as u32,
                td_core::layout::Page::Dashboard { .. } => 0,
            })
            .sum();
        (layout.pages.len() as u32, tiles)
    }

    #[test]
    fn crc32_matches_the_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_put_lands_in_order_and_replies_with_the_layout_state() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let layout = dev();
        let report = put_layout(&session, Orientation::Landscape, &layout).unwrap();
        let (pages, tiles) = counts(&layout);
        assert_eq!(
            (
                report.state.source.as_str(),
                report.state.name.as_str(),
                report.state.pages,
                report.state.tiles
            ),
            ("storage", layout.name.as_str(), pages, tiles)
        );
        assert_eq!(report.retransmits, 0);
        assert_eq!(report.chunks, chunk_count(layout.to_canonical_json().len()));
        assert_eq!(
            deck.layout(Orientation::Landscape).unwrap(),
            layout.to_canonical_json()
        );
        let put = deck
            .received()
            .into_iter()
            .find(|m| m["m"] == "layout.put")
            .unwrap();
        assert_eq!(put["crc32"], crc32(&layout.to_canonical_json()));
    }

    #[test]
    fn displayed_layout_state_ignores_inactive_pushes_and_saved_orientation_until_reboot() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let fixture = |orientation: &str| {
            Layout::parse(
                &serde_json::to_vec(&json!({
                    "schema": 1, "name": orientation, "orientation": orientation,
                    "pages": [{"id": "shared", "title": "Fixture", "kind": "grid",
                        "rows": 1, "cols": 1, "tiles": [{"id": "action", "label": "Harmless",
                            "row": 0, "col": 0, "rowSpan": 1, "colSpan": 1, "icon": null,
                            "action": {"type": "none"}}]}]
                }))
                .unwrap(),
            )
            .unwrap()
        };
        let landscape = fixture("landscape");
        let portrait = fixture("portrait");
        put_layout(&session, Orientation::Landscape, &landscape).unwrap();
        let inactive = put_layout(&session, Orientation::Portrait, &portrait).unwrap();
        assert_eq!(inactive.state.name, landscape.name);
        assert_eq!(inactive.state.orientation, Orientation::Landscape);
        session
            .request(
                &Request::SettingsPut(td_core::Settings {
                    orientation: Orientation::Portrait,
                    ..td_core::Settings::default()
                }),
                Duration::from_secs(1),
            )
            .unwrap();
        let shown = session.layout_state(Duration::from_secs(1)).unwrap();
        assert_eq!(shown.name, landscape.name);
        assert_eq!(shown.orientation, Orientation::Landscape);
        session.send_line("reboot").unwrap();
        let shown = session.layout_state(Duration::from_secs(1)).unwrap();
        assert_eq!(shown.name, portrait.name);
        assert_eq!(shown.orientation, Orientation::Portrait);
    }

    #[test]
    fn a_lost_chunk_goes_again() {
        let deck = MockDeck::new();
        deck.lose_chunks([1, 1, 3]);
        let session = open(&deck);
        let layout = dev();
        let report = put_layout(&session, Orientation::Landscape, &layout).unwrap();
        assert!(report.retransmits >= 2, "{report:?}");
        assert_eq!(
            deck.layout(Orientation::Landscape).unwrap(),
            layout.to_canonical_json()
        );
    }

    #[test]
    fn five_lost_transmissions_abort_the_put() {
        let deck = MockDeck::new();
        deck.lose_chunks([0; 5]);
        let session = open(&deck);
        assert_eq!(
            put_layout(&session, Orientation::Landscape, &dev()),
            Err(XferError::Aborted { idx: 0 })
        );
        assert_eq!(deck.layout(Orientation::Landscape), None);
        assert!(
            !session.is_open(),
            "an abandoned Device transfer closes its session"
        );
    }

    #[test]
    fn a_bad_crc_is_refused_and_keeps_the_old_layout() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let layout = dev();
        put_layout(&session, Orientation::Landscape, &layout).unwrap();
        deck.corrupt_next_put();
        let error = put_layout(&session, Orientation::Landscape, &layout).unwrap_err();
        assert!(error.to_string().contains("crc32"), "{error}");
        assert_eq!(
            deck.layout(Orientation::Landscape).unwrap(),
            layout.to_canonical_json()
        );
    }

    #[test]
    fn an_oversize_layout_never_leaves_the_app() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let data = vec![b' '; UPLOAD_MAX + 1];
        assert_eq!(
            put_bytes(&session, Orientation::Landscape, &data),
            Err(XferError::TooLarge(UPLOAD_MAX + 1))
        );
        assert!(deck.received().is_empty());
    }

    #[test]
    fn mismatched_layout_writes_are_refused_before_a_transfer_starts() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let layout = dev();
        for result in [
            put_layout(&session, Orientation::Portrait, &layout),
            put_bytes(&session, Orientation::Portrait, &layout.to_canonical_json()),
        ] {
            let error = result.expect_err("a landscape layout must not be written to portrait");
            let message = error.to_string();
            assert!(
                message.contains("landscape") && message.contains("portrait"),
                "{message}"
            );
        }
        assert!(deck.received().is_empty());
        assert!(deck.layout(Orientation::Portrait).is_none());
    }

    #[test]
    fn an_invalid_layout_is_refused_at_xfer_end() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let error = put_bytes(&session, Orientation::Landscape, b"{\"schema\":2}").unwrap_err();
        assert!(error.to_string().contains("shape."), "{error}");
        assert_eq!(deck.layout(Orientation::Landscape), None);
    }

    #[test]
    fn a_get_returns_the_bytes_the_put_stored() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let mut layout = dev();
        layout.orientation = Orientation::Portrait;
        put_layout(&session, Orientation::Portrait, &layout).unwrap();
        let report = get_layout(&session, Orientation::Portrait).unwrap();
        assert_eq!(report.data, layout.to_canonical_json());
        assert_eq!(report.crc32, crc32(&report.data));
        assert!(
            get_layout(&session, Orientation::Landscape)
                .unwrap_err()
                .to_string()
                .contains("no stored layout")
        );
    }

    #[test]
    fn a_get_survives_lost_chunks() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let layout = dev();
        put_layout(&session, Orientation::Landscape, &layout).unwrap();
        deck.lose_sent_chunks([0, 2]);
        let report = get_layout(&session, Orientation::Landscape).unwrap();
        assert_eq!(report.data, layout.to_canonical_json());
    }

    #[test]
    fn a_32_kb_put_and_get_round_trip() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let mut layout = dev();
        let pad = |layout: &mut Layout, text: String| {
            if let td_core::layout::Page::Grid { tiles, .. } = &mut layout.pages[1]
                && let td_core::layout::Action::Launch { cwd, .. } = &mut tiles[0].action
            {
                *cwd = text;
            }
        };
        pad(&mut layout, String::new());
        let base = layout.to_canonical_json().len();
        pad(&mut layout, "a".repeat(UPLOAD_MAX - base));
        let bytes = layout.to_canonical_json();
        assert_eq!(bytes.len(), UPLOAD_MAX);
        let put = put_layout(&session, Orientation::Landscape, &layout).unwrap();
        assert_eq!(put.chunks, 33);
        assert_eq!(
            get_layout(&session, Orientation::Landscape).unwrap().data,
            bytes
        );
    }

    fn icon(seed: u8) -> (String, Vec<u8>) {
        let mut file = b"TDI1".to_vec();
        file.resize(ICON_BYTES, seed);
        (format!("{seed:016x}"), file)
    }

    #[test]
    fn a_second_push_sends_no_icon_the_deck_already_has() {
        let deck = MockDeck::new();
        let session = open(&deck);
        let files: std::collections::BTreeMap<String, Vec<u8>> = (1..=3).map(icon).collect();
        let wanted: BTreeSet<String> = files.keys().cloned().collect();
        let first = push_icons(&session, &wanted, |id| files.get(id).cloned()).unwrap();
        assert_eq!(first.sent.len(), 3);
        let second = push_icons(&session, &wanted, |id| files.get(id).cloned()).unwrap();
        assert_eq!(second, IconSync::default());
        assert_eq!(deck.icons(), wanted);
        assert_eq!(
            deck.icon_file("0000000000000002"),
            files.get("0000000000000002").cloned()
        );
    }

    #[test]
    fn prune_removes_only_unnamed_icons_and_the_push_sends_the_missing_one() {
        let deck = MockDeck::new();
        deck.add_icons((1..=40u8).map(|seed| icon(seed).0));
        let session = open(&deck);
        let keep: BTreeSet<String> = [icon(2).0, icon(35).0, icon(50).0].into();
        let removed = prune_icons(&session, &keep).unwrap();
        assert_eq!(removed.len(), 38);
        assert!(removed.iter().all(|id| !keep.contains(id)));
        assert_eq!(deck.icons(), [icon(2).0, icon(35).0].into());
        let sync =
            push_icons(&session, &keep, |id| (id == icon(50).0).then(|| icon(50).1)).unwrap();
        assert_eq!(sync.sent, vec![icon(50).0]);
        assert_eq!(deck.icons(), keep);
    }

    #[test]
    fn the_deck_refuses_an_icon_of_the_wrong_size_or_id() {
        let deck = MockDeck::new();
        let session = open(&deck);
        assert!(matches!(
            put_icon(&session, &icon(1).0, b"TDI1"),
            Err(XferError::Size { .. })
        ));
        assert!(put_icon(&session, "NOT-AN-ID", &icon(1).1).is_err());
        assert!(remove_icon(&session, &icon(1).0).is_err());
        assert!(deck.icons().is_empty());
    }
}
