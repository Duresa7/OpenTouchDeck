//! Packs an RGBA bitmap into the deck's `TDI1` icon file, per `schema/icon-format.md`: a
//! 12-byte header, 64 by 64 RGB565 pixels, then 64 by 64 A8 values. Pure, so the tests can
//! hold it to the golden file byte for byte.

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use resvg::{tiny_skia, usvg};
use sha2::{Digest, Sha256};

pub const SIDE: u32 = 64;
pub const HEADER_BYTES: usize = 12;
pub const COLOR_BYTES: usize = (SIDE * SIDE * 2) as usize;
pub const ALPHA_BYTES: usize = (SIDE * SIDE) as usize;
pub const FILE_BYTES: usize = HEADER_BYTES + COLOR_BYTES + ALPHA_BYTES;
const MAGIC: &[u8; 4] = b"TDI1";
const FORMAT_RGB565A8: u16 = 1;

/// Packs a 64 by 64 image. Each channel is truncated to its high bits and alpha is copied.
pub fn pack(image: &RgbaImage) -> Vec<u8> {
    assert_eq!(
        image.dimensions(),
        (SIDE, SIDE),
        "pack takes a 64 by 64 image"
    );
    let mut file = Vec::with_capacity(FILE_BYTES);
    file.extend_from_slice(MAGIC);
    for value in [SIDE as u16, SIDE as u16, FORMAT_RGB565A8, 0] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    for Rgba([r, g, b, _]) in image.pixels() {
        let rgb565 = (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
        file.extend_from_slice(&rgb565.to_le_bytes());
    }
    file.extend(image.pixels().map(|Rgba([_, _, _, a])| *a));
    file
}

/// Scales an image of any size to fit 64 by 64, keeping its aspect, and centres it on a
/// transparent square.
pub fn fit(image: &RgbaImage) -> RgbaImage {
    if image.dimensions() == (SIDE, SIDE) {
        return image.clone();
    }
    fit_tile(image, SIDE, SIDE, 0, None, None)
}

/// Fits and packs an image of any size.
pub fn pack_any(image: &RgbaImage) -> Vec<u8> {
    pack(&fit(image))
}

/// The icon's id: the first 16 hex digits of the file's SHA-256.
pub fn icon_id(file: &[u8]) -> String {
    Sha256::digest(file)[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `file` is a `TDI1` icon the deck draws.
pub fn is_icon(file: &[u8]) -> bool {
    file.len() == FILE_BYTES && file.starts_with(MAGIC) && file[4..12] == [64, 0, 64, 0, 1, 0, 0, 0]
}

/// Renders an SVG into a `side` by `side` image, scaled to fit and centred.
pub fn render_svg(svg: &[u8], side: u32) -> Result<RgbaImage, String> {
    render_svg_size(svg, side, side)
}

/// Renders SVG directly into a rectangular canvas without losing its aspect ratio.
pub fn render_svg_size(svg: &[u8], width: u32, height: u32) -> Result<RgbaImage, String> {
    render_svg_fit(svg, width, height, false)
}

/// Renders directly to the canvas, optionally cropping to cover it.
pub fn render_svg_fit(
    svg: &[u8],
    width: u32,
    height: u32,
    cover: bool,
) -> Result<RgbaImage, String> {
    draw(&parse_svg(svg)?, width, height, cover)
}

fn draw(tree: &usvg::Tree, width: u32, height: u32, cover: bool) -> Result<RgbaImage, String> {
    let size = tree.size();
    let sx = width as f32 / size.width();
    let sy = height as f32 / size.height();
    let scale = if cover { sx.max(sy) } else { sx.min(sy) };
    let dx = (width as f32 - size.width() * scale) / 2.0;
    let dy = (height as f32 - size.height() * scale) / 2.0;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("svg: empty canvas")?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
        &mut pixmap.as_mut(),
    );
    let mut image = RgbaImage::new(width, height);
    for (pixel, out) in pixmap.pixels().iter().zip(image.pixels_mut()) {
        let color = pixel.demultiply();
        *out = Rgba([color.red(), color.green(), color.blue(), color.alpha()]);
    }
    Ok(image)
}

/// The ink monochrome icons draw with: the stroke colour of the built-in Lucide icons. An SVG
/// that paints with `currentColor` gets it through its root `color` attribute.
pub const INK: &str = "#f2f5fa";

/// The most XML nodes one SVG may hold. Icons hold a few hundred.
const MAX_SVG_NODES: u32 = 200_000;
/// The largest `.svgz` once decompressed.
const MAX_SVGZ: u64 = 16 << 20;

/// Where an SVG comes from, which sets what it may hold. Neither kind reads anything outside
/// the document, and neither draws embedded `data:` images: resvg is built without raster
/// decoders, and a nested SVG image draws nothing either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgSource {
    /// A catalog or Icon pack icon. A DOCTYPE is fine, but an `ENTITY` declaration is not, so
    /// no entity can expand.
    Catalog,
    /// A file someone uploaded: no DOCTYPE, no ENTITY and no compression.
    Upload,
}

/// Parses a catalog or Icon pack SVG. See [`SvgSource::Catalog`].
pub fn parse_svg(svg: &[u8]) -> Result<usvg::Tree, String> {
    parse_svg_from(svg, SvgSource::Catalog)
}

pub fn parse_svg_from(svg: &[u8], source: SvgSource) -> Result<usvg::Tree, String> {
    let unzipped;
    let svg = if svg.starts_with(&[0x1f, 0x8b]) {
        if source == SvgSource::Upload {
            return Err("svg: compressed SVGs aren't accepted".into());
        }
        unzipped = gunzip(svg)?;
        &unzipped[..]
    } else {
        svg
    };
    let text = std::str::from_utf8(svg).map_err(|_| "svg: not UTF-8".to_owned())?;
    let document = parse_xml(text, source)?;
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    usvg::Tree::from_xmltree(&document, &options).map_err(|error| format!("svg: {error}"))
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data)
        .take(MAX_SVGZ + 1)
        .read_to_end(&mut out)
        .map_err(|error| format!("svg: {error}"))?;
    if out.len() as u64 > MAX_SVGZ {
        return Err("svg: the SVG is larger than 16 MiB once decompressed".into());
    }
    Ok(out)
}

fn parse_xml(text: &str, source: SvgSource) -> Result<roxmltree::Document<'_>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if contains_ignore_case(text, b"<!entity") {
        return Err("svg: ENTITY declarations aren't accepted".into());
    }
    if source == SvgSource::Upload && contains_ignore_case(text, b"<!doctype") {
        return Err("svg: DOCTYPE declarations aren't accepted".into());
    }
    // A DTD that declares no entity can't expand anything, and none is ever fetched.
    let options = roxmltree::ParsingOptions {
        allow_dtd: source == SvgSource::Catalog,
        nodes_limit: MAX_SVG_NODES,
    };
    roxmltree::Document::parse_with_options(text, options).map_err(|error| format!("svg: {error}"))
}

/// Whether `text` holds `needle`, which is lowercase ASCII, in any case.
fn contains_ignore_case(text: &str, needle: &[u8]) -> bool {
    text.as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Renders an uploaded SVG so its longer side is `max` pixels, keeping its aspect. A root
/// without a `color` draws `currentColor` in [`INK`], the way a pack icon does. See
/// [`SvgSource::Upload`] for what it may hold.
pub fn render_svg_upload(svg: &[u8], max: u32) -> Result<RgbaImage, String> {
    if svg.starts_with(&[0x1f, 0x8b]) {
        return Err("svg: compressed SVGs aren't accepted".into());
    }
    let text = std::str::from_utf8(svg).map_err(|_| "svg: not UTF-8".to_owned())?;
    let inked = with_root_color(text)?;
    let tree = parse_svg_from(inked.as_bytes(), SvgSource::Upload)?;
    let size = tree.size();
    let scale = max as f32 / size.width().max(size.height());
    let width = ((size.width() * scale).round() as u32).clamp(1, max);
    let height = ((size.height() * scale).round() as u32).clamp(1, max);
    draw(&tree, width, height, false)
}

/// Adds `color` to the root element when it sets none. The root is found by parsing, so an
/// `<svg` inside a comment is never mistaken for it.
fn with_root_color(svg: &str) -> Result<String, String> {
    let svg = svg.strip_prefix('\u{feff}').unwrap_or(svg);
    let document = parse_xml(svg, SvgSource::Upload)?;
    let root = document.root_element();
    if root.attribute("color").is_some() {
        return Ok(svg.to_owned());
    }
    let start = root.range().start + 1;
    let name = svg[start..]
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .ok_or("svg: the root element isn't closed")?;
    let at = start + name;
    Ok(format!("{} color=\"{INK}\"{}", &svg[..at], &svg[at..]))
}

/// Contains an image inside a padded Tile, then applies tint and composites its background.
/// Callers validate nonzero dimensions and padding before allocating.
pub fn fit_tile(
    image: &RgbaImage,
    width: u32,
    height: u32,
    padding: u32,
    background: Option<[u8; 4]>,
    tint: Option<[u8; 3]>,
) -> RgbaImage {
    let available_w = width - padding * 2;
    let available_h = height - padding * 2;
    let scale =
        (available_w as f64 / image.width() as f64).min(available_h as f64 / image.height() as f64);
    let w = ((image.width() as f64 * scale).round() as u32).clamp(1, available_w);
    let h = ((image.height() as f64 * scale).round() as u32).clamp(1, available_h);
    let mut scaled = if image.dimensions() == (w, h) {
        image.clone()
    } else {
        imageops::resize(image, w, h, FilterType::Lanczos3)
    };
    if let Some([r, g, b]) = tint {
        for pixel in scaled.pixels_mut() {
            pixel.0[..3].copy_from_slice(&[r, g, b]);
        }
    }
    let mut tile = RgbaImage::from_pixel(width, height, Rgba(background.unwrap_or([0; 4])));
    imageops::overlay(
        &mut tile,
        &scaled,
        i64::from((width - w) / 2),
        i64::from((height - h) / 2),
    );
    tile
}
