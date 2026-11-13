//! The value of one live stream, as `v2.live.set` carries it and as an Integration reports it.

use serde::{Serialize, Serializer};
use unicode_normalization::UnicodeNormalization;

use crate::layout::StreamType;

/// The largest magnitude a number in a live value may have.
pub const VALUE_MAX: f64 = 1e9;

/// A live value, typed as a Tile stream declares it.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveValue {
    Number(f64),
    Series(Vec<f64>),
    Text(String),
    /// An Asset reference `sha256:<id>`, or `None` to clear the picture.
    Image(Option<String>),
}

impl LiveValue {
    pub fn kind(&self) -> StreamType {
        match self {
            LiveValue::Number(_) => StreamType::Number,
            LiveValue::Series(_) => StreamType::Series,
            LiveValue::Text(_) => StreamType::Text,
            LiveValue::Image(_) => StreamType::Image,
        }
    }

    /// Whether a Device with these caps accepts the value's shape: numbers finite and within ±1e9, a series of 1
    /// to `series` of them, and text of at most `text_bytes` bytes.
    pub fn fits(&self, series: usize, text_bytes: usize) -> bool {
        let number = |v: &f64| v.is_finite() && v.abs() <= VALUE_MAX;
        match self {
            LiveValue::Number(v) => number(v),
            LiveValue::Series(values) => {
                !values.is_empty() && values.len() <= series && values.iter().all(number)
            }
            LiveValue::Text(text) => text_bytes > 0 && text.len() <= text_bytes,
            LiveValue::Image(_) => true,
        }
    }
}

const LATIN_EXTENDED_A: [&str; 128] = [
    "A", "a", "A", "a", "A", "a", "C", "c", "C", "c", "C", "c", "C", "c", "D", "d", "D", "d", "E",
    "e", "E", "e", "E", "e", "E", "e", "E", "e", "G", "g", "G", "g", "G", "g", "G", "g", "H", "h",
    "H", "h", "I", "i", "I", "i", "I", "i", "I", "i", "I", "i", "IJ", "ij", "J", "j", "K", "k",
    "k", "L", "l", "L", "l", "L", "l", "L", "l", "L", "l", "N", "n", "N", "n", "N", "n", "'n", "N",
    "n", "O", "o", "O", "o", "O", "o", "OE", "oe", "R", "r", "R", "r", "R", "r", "S", "s", "S",
    "s", "S", "s", "S", "s", "T", "t", "T", "t", "T", "t", "U", "u", "U", "u", "U", "u", "U", "u",
    "U", "u", "U", "u", "W", "w", "Y", "y", "Y", "Z", "z", "Z", "z", "Z", "z", "s",
];

/// Text an Integration reports, made fit for a Device that draws `max_bytes` of the glyph set in `docs/design.md`
/// (Fonts on the Device), by the rules and vectors in `schema/live/text-vectors.json`.
pub fn fit_text(text: &str, max_bytes: usize) -> String {
    let mut folded = String::with_capacity(text.len());
    for c in text.nfc() {
        match c {
            '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}' => folded.push(' '),
            '\u{200b}'..='\u{200d}'
            | '\u{2060}'
            | '\u{feff}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{300}'..='\u{36f}' => {}
            '\u{100}'..='\u{17f}' => folded.push_str(LATIN_EXTENDED_A[c as usize - 0x100]),
            '\u{218}' => folded.push('S'),
            '\u{219}' => folded.push('s'),
            '\u{21a}' => folded.push('T'),
            '\u{21b}' => folded.push('t'),
            '\u{2010}'..='\u{2012}' | '\u{2015}' => folded.push('\u{2013}'),
            '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => folded.push(' '),
            '\u{2039}' => folded.push('<'),
            '\u{203a}' => folded.push('>'),
            c => folded.push(c),
        }
    }
    let mut fitted = String::with_capacity(max_bytes.min(folded.len()));
    for word in folded.split_whitespace() {
        let gap = usize::from(!fitted.is_empty());
        if fitted.len() + gap >= max_bytes {
            break;
        }
        if gap == 1 {
            fitted.push(' ');
        }
        for c in word.chars() {
            if fitted.len() + c.len_utf8() > max_bytes {
                return fitted.trim_end_matches(' ').to_owned();
            }
            fitted.push(c);
        }
    }
    fitted.trim_end_matches(' ').to_owned()
}

/// A whole number goes on the wire without a fraction, as `schema/protocol/golden-frames-v2.json` writes it.
fn wire_number<S: Serializer>(value: f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() <= VALUE_MAX {
        serializer.serialize_i64(value as i64)
    } else {
        serializer.serialize_f64(value)
    }
}

impl Serialize for LiveValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct Number(f64);
        impl Serialize for Number {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                wire_number(self.0, serializer)
            }
        }
        match self {
            LiveValue::Number(v) => wire_number(*v, serializer),
            LiveValue::Series(values) => serializer.collect_seq(values.iter().map(|v| Number(*v))),
            LiveValue::Text(text) => serializer.serialize_str(text),
            LiveValue::Image(image) => image.serialize(serializer),
        }
    }
}

/// A screen's size in pixels, in the orientation the Layout uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenSize {
    pub w: u16,
    pub h: u16,
}

/// A Tile's rect in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub w: u16,
    pub h: u16,
}

/// Where a Tile sits on a Device's screen, as the Firmware draws it (`ui_geometry` and `ui_tile_rect` in
/// `firmware/components/td_ui_core`). None for a missing or non-grid Page, a missing Tile, an invalid grid or span,
/// or an empty screen.
pub fn tile_rect(
    layout: &crate::Layout,
    page_id: &str,
    tile_id: &str,
    screen: ScreenSize,
) -> Option<PixelRect> {
    layout.pages.iter().find_map(|page| match page {
        crate::layout::Page::Grid {
            id,
            rows,
            cols,
            tiles,
            ..
        } if id == page_id => {
            let tile = tiles.iter().find(|tile| tile.id == tile_id)?;
            cell_rect(
                screen,
                layout.full_screen,
                [*rows, *cols],
                [tile.row, tile.col, tile.row_span, tile.col_span],
            )
        }
        _ => None,
    })
}

/// The rect of a cell span `[row, col, row_span, col_span]` on a `[rows, cols]` grid, by the rules and vectors in
/// `schema/live/tile-geometry-vectors.json`.
pub fn cell_rect(
    screen: ScreenSize,
    full_screen: bool,
    grid: [u8; 2],
    cell: [u8; 4],
) -> Option<PixelRect> {
    const GRID_MAX: u32 = 8;
    let (w, h) = (u32::from(screen.w), u32::from(screen.h));
    let [rows, cols] = grid.map(u32::from);
    let [row, col, row_span, col_span] = cell.map(u32::from);
    let fits = |n| (1..=GRID_MAX).contains(&n);
    if w == 0
        || h == 0
        || !fits(rows)
        || !fits(cols)
        || row_span == 0
        || col_span == 0
        || row + row_span > rows
        || col + col_span > cols
    {
        return None;
    }
    let scaled = |px: u32| px * w.min(h) / 600;
    let (x, y, area_w, area_h) = if full_screen {
        (0, 0, w, h)
    } else {
        let title = scaled(56);
        let pager = scaled(36);
        let margin_x = scaled(28);
        let margin_y = scaled(20);
        let band_h = h.checked_sub(title + pager)?;
        (
            margin_x,
            title + margin_y,
            w.checked_sub(2 * margin_x)?,
            band_h.checked_sub(2 * margin_y)?,
        )
    };
    let gap = scaled(16);
    let edge = |length: u32, cells: u32, n: u32| n * (length + gap) / cells;
    let x0 = edge(area_w, cols, col);
    let x1 = edge(area_w, cols, col + col_span);
    let y0 = edge(area_h, rows, row);
    let y1 = edge(area_h, rows, row + row_span);
    Some(PixelRect {
        x: i32::try_from(x + x0).ok()?,
        y: i32::try_from(y + y0).ok()?,
        w: u16::try_from((x1 - x0).checked_sub(gap)?).ok()?,
        h: u16::try_from((y1 - y0).checked_sub(gap)?).ok()?,
    })
}

/// The side of the square cover a `now-playing` Tile of this size draws, or None when it has no art box, by the
/// rules and vectors in `schema/live/now-playing-vectors.json`.
pub fn art_size(tile_w: u16, tile_h: u16) -> Option<u16> {
    now_playing_layout(tile_w, tile_h).art.map(|r| r.w)
}

impl ScreenSize {
    /// Orient native panel dimensions for the Layout.
    pub fn oriented(self, orientation: crate::layout::Orientation) -> Self {
        let (short, long) = (self.w.min(self.h), self.w.max(self.h));
        match orientation {
            crate::layout::Orientation::Portrait => Self { w: short, h: long },
            crate::layout::Orientation::Landscape => Self { w: long, h: short },
        }
    }
}

/// Inverse of cell edges, including the following gap. Outside indices are retained.
/// Returns None for invalid grids, screens or non-finite pointer coordinates.
pub fn cell_at(
    screen: ScreenSize,
    full_screen: bool,
    grid: [u8; 2],
    x: f64,
    y: f64,
) -> Option<[i32; 2]> {
    let area = cell_rect(screen, full_screen, grid, [0, 0, grid[0], grid[1]])?;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let gap = f64::from(u32::from(screen.w.min(screen.h)) * 16 / 600);
    let index = |point: f64, origin: i32, length: u16, cells: u8| {
        (((point - f64::from(origin)).floor() + 1.0) * f64::from(cells) - 1.0)
            / (f64::from(length) + gap)
    };
    Some([
        index(y, area.y, area.h, grid[0]).floor() as i32,
        index(x, area.x, area.w, grid[1]).floor() as i32,
    ])
}

/// Tile-local now-playing geometry. There is no progress bar on the Device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlayingLayout {
    pub art: Option<PixelRect>,
    pub text_x: i32,
    pub text_w: u16,
    pub controls: bool,
    /// Wire part name and its half-open rectangle.
    pub parts: Vec<(&'static str, PixelRect)>,
}

pub fn now_playing_layout(w: u16, h: u16) -> NowPlayingLayout {
    let size = h
        .saturating_sub(40)
        .min((u32::from(w.saturating_sub(40)) * 2 / 5) as u16)
        .min(256);
    let art = (size >= 48).then_some(PixelRect {
        x: 20,
        y: i32::from((h - size) / 2),
        w: size,
        h: size,
    });
    let text_x = art.map_or(20, |r| r.x + i32::from(r.w) + 20);
    let text_w = w.saturating_sub(20 + text_x as u16);
    let controls = h >= 200 && text_w >= 180;
    let parts = if controls {
        let each = text_w / 3;
        ["previous", "play_pause", "next"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    name,
                    PixelRect {
                        x: text_x + i as i32 * i32::from(each),
                        y: i32::from(h) - 76,
                        w: if i == 2 { text_w - 2 * each } else { each },
                        h: 56,
                    },
                )
            })
            .collect()
    } else {
        vec![("play_pause", PixelRect { x: 0, y: 0, w, h })]
    };
    NowPlayingLayout {
        art,
        text_x,
        text_w,
        controls,
        parts,
    }
}

/// Centered source crop for a cover; smaller images retain their native size.
pub fn cover_crop(source: ScreenSize, art: PixelRect) -> PixelRect {
    let (w, h) = (source.w.min(art.w), source.h.min(art.h));
    PixelRect {
        x: i32::from((source.w - w) / 2),
        y: i32::from((source.h - h) / 2),
        w,
        h,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarLayout {
    pub glyph: PixelRect,
    pub track: PixelRect,
    pub capture: Option<PixelRect>,
    pub mute: Option<PixelRect>,
}

pub fn bar_layout(w: u16, h: u16, muted_declared: bool) -> BarLayout {
    let glyph = PixelRect {
        x: 20,
        y: 20,
        w: 32,
        h: 32,
    };
    let track = PixelRect {
        x: 20,
        y: i32::from(h) - 48,
        w: w.saturating_sub(40),
        h: 28,
    };
    let zones = w >= 72 && h >= 72;
    let top = (track.y - 24).max(20);
    let bottom = (track.y + 52).min(i32::from(h) - 20);
    BarLayout {
        glyph,
        track,
        capture: zones.then_some(PixelRect {
            x: 20,
            y: top,
            w: track.w,
            h: (bottom - top).max(0) as u16,
        }),
        mute: (zones && muted_declared).then_some(glyph),
    }
}

impl PixelRect {
    /// Half-open Device or Tile-local bounds.
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.w)
            && i64::from(y) < i64::from(self.y) + i64::from(self.h)
    }
}

impl NowPlayingLayout {
    /// Idle or unknown playback state has no actionable part.
    pub fn part_at(&self, x: i32, y: i32, state: u8) -> Option<&'static str> {
        if !matches!(state, 1 | 2) {
            return None;
        }
        self.parts
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(part, _)| *part)
    }
}

impl BarLayout {
    pub fn zone_at(&self, x: i32, y: i32, fresh: bool, input: bool) -> Option<&'static str> {
        if !fresh {
            return None;
        }
        if self.mute.is_some_and(|r| r.contains(x, y)) {
            return Some("mute");
        }
        (input && self.capture.is_some_and(|r| r.contains(x, y))).then_some("value")
    }
}
