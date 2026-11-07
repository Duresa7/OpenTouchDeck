//! What a catalog SVG and an uploaded SVG may hold: no entity expansion, and no embedded
//! images drawn in either.

use base64::Engine;
use std::io::Write;
use td_host::icon_pack::{
    INK, SvgSource, parse_svg, parse_svg_from, render_svg_fit, render_svg_upload,
};

const SVG_NS: &str = "xmlns=\"http://www.w3.org/2000/svg\"";

fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(width, height, image::Rgba(rgba))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// A left half in `currentColor`, and `image` drawn over the right half.
fn with_image(href: &str) -> String {
    format!(
        "<svg {SVG_NS} viewBox=\"0 0 20 20\"><rect width=\"10\" height=\"20\" fill=\"currentColor\"/>\
         <image x=\"10\" width=\"10\" height=\"20\" href=\"{href}\"/></svg>"
    )
}

fn right_alpha(image: &image::RgbaImage) -> u8 {
    image.get_pixel(15, 10).0[3]
}

#[test]
fn entity_expansion_is_refused_before_it_starts() {
    // The billion-laughs shape, kept small: a decoy `<svg` in a comment, a DTD that declares an
    // entity, and many references to it.
    let refs = "&a;".repeat(2_000);
    let payload = format!(
        "<!-- <svg --><!DOCTYPE svg [<!ENTITY a \"{}\">]><svg {SVG_NS}><text>{refs}</text></svg>",
        "x".repeat(1024)
    );
    let upload = render_svg_upload(payload.as_bytes(), 64).unwrap_err();
    assert!(upload.contains("ENTITY"), "{upload}");
    let lower = payload
        .replace("<!DOCTYPE", "<!doctype")
        .replace("<!ENTITY", "<!entity");
    assert!(render_svg_upload(lower.as_bytes(), 64).is_err());
    // Catalog icons refuse an internal DTD subset too, so no entity can expand there either.
    assert!(parse_svg(payload.as_bytes()).is_err());
}

#[test]
fn a_plain_doctype_is_fine_in_a_catalog_icon_but_not_in_an_upload() {
    let svg = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \
         \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\
         <svg {SVG_NS} viewBox=\"0 0 10 10\"><rect width=\"10\" height=\"10\"/></svg>"
    );
    assert!(parse_svg(svg.as_bytes()).is_ok());
    assert!(parse_svg_from(svg.as_bytes(), SvgSource::Upload).is_err());
    assert!(render_svg_upload(svg.as_bytes(), 64).is_err());
}

#[test]
fn uploads_draw_no_embedded_images() {
    let red = png(4, 4, [255, 0, 0, 255]);
    let nested = format!(
        "<svg {SVG_NS} viewBox=\"0 0 1 1\"><rect width=\"1\" height=\"1\" fill=\"red\"/></svg>"
    );
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(nested.as_bytes()).unwrap();
    let gz = gz.finish().unwrap();
    for href in [
        data_url("image/png", &red),
        data_url("image/svg+xml", nested.as_bytes()),
        data_url("image/svg+xml", &gz),
        data_url("text/plain", &red),
    ] {
        let image = render_svg_upload(with_image(&href).as_bytes(), 20).unwrap();
        assert_eq!(right_alpha(&image), 0, "{}", &href[..30]);
        // The rest of the upload still draws, in the ink.
        let ink = u32::from_str_radix(&INK[1..], 16).unwrap().to_be_bytes();
        assert_eq!(image.get_pixel(5, 10).0, [ink[1], ink[2], ink[3], 255]);
    }
}

#[test]
fn catalog_icons_draw_no_embedded_images_either() {
    // A PNG whose header claims 20000 by 20000 pixels, a nested SVG (plain and gzipped) and a
    // JPEG: none is decoded, and the rest of the icon still draws.
    let mut huge = png(1, 1, [255, 0, 0, 255]);
    huge[16..20].copy_from_slice(&20_000u32.to_be_bytes());
    huge[20..24].copy_from_slice(&20_000u32.to_be_bytes());
    let nested = format!(
        "<svg {SVG_NS} viewBox=\"0 0 1 1\"><rect width=\"1\" height=\"1\" fill=\"red\"/></svg>"
    );
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(nested.as_bytes()).unwrap();
    for href in [
        data_url("image/png", &png(4, 4, [255, 0, 0, 255])),
        data_url("image/png", &huge),
        data_url("image/svg+xml", nested.as_bytes()),
        data_url("image/svg+xml", &gz.finish().unwrap()),
        data_url("image/jpeg", b"\xff\xd8\xff"),
    ] {
        let image = render_svg_fit(with_image(&href).as_bytes(), 20, 20, false).unwrap();
        assert_eq!(right_alpha(&image), 0, "{}", &href[..30]);
        assert_eq!(image.get_pixel(5, 10).0[3], 255);
    }
}

#[test]
fn an_upload_gets_the_ink_on_its_real_root_and_refuses_compression() {
    let svg = format!(
        "<!-- <svg color=\"red\"> --><svg {SVG_NS} viewBox=\"0 0 10 10\">\
         <rect width=\"10\" height=\"10\" fill=\"currentColor\"/></svg>"
    );
    let image = render_svg_upload(svg.as_bytes(), 10).unwrap();
    let ink = u32::from_str_radix(&INK[1..], 16).unwrap().to_be_bytes();
    assert_eq!(image.get_pixel(5, 5).0, [ink[1], ink[2], ink[3], 255]);
    // A root that sets its own colour keeps it.
    let own = svg.replace("viewBox", "color=\"#00ff00\" viewBox");
    assert_eq!(
        render_svg_upload(own.as_bytes(), 10)
            .unwrap()
            .get_pixel(5, 5)
            .0,
        [0, 255, 0, 255]
    );

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(svg.as_bytes()).unwrap();
    assert!(render_svg_upload(&gz.finish().unwrap(), 10).is_err());
}
