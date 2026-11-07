use std::io::{Cursor, Read, Write};
use td_link::bundle::Bundle;
const FIXTURE: &[u8] = include_bytes!("fixtures/touchdeck-test.tdfw");
fn changed(name: &str, replacement: Option<&[u8]>) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(Cursor::new(FIXTURE)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for idx in 0..input.len() {
        let mut file = input.by_index(idx).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        let filename = file.name().unwrap();
        let bytes = if filename.as_ref() == name {
            match replacement {
                Some(bytes) => bytes,
                None => continue,
            }
        } else {
            &data
        };
        out.start_file(filename, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(bytes).unwrap();
    }
    out.finish().unwrap().into_inner()
}
#[test]
fn fixture_and_manifest_offsets() {
    let b = Bundle::parse(FIXTURE).unwrap();
    assert_eq!(b.manifest.version, "test-only");
    assert_eq!(
        b.images.iter().map(|i| i.offset).collect::<Vec<_>>(),
        [0, 0x8000, 0x40000]
    );
}
#[test]
fn missing_bootloader_is_refused() {
    assert!(Bundle::parse(&changed("bootloader.bin", None)).is_err());
}
#[test]
fn corrupt_sha_is_refused() {
    assert!(
        Bundle::parse(&changed("touchdeck.bin", Some(b"corrupt")))
            .unwrap_err()
            .contains("sha256")
    );
}
#[test]
fn unsafe_offsets_are_refused() {
    let b = Bundle::parse(FIXTURE).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(FIXTURE)).unwrap();
    let mut text = String::new();
    z.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let text = text.replace("0x40000", "0x840000");
    assert_eq!(b.images[2].offset, 0x40000);
    assert!(
        Bundle::parse(&changed("manifest.json", Some(text.as_bytes())))
            .unwrap_err()
            .contains("unsafe offset")
    );
}
fn with_version(version: &str) -> Vec<u8> {
    let mut text = String::new();
    zip::ZipArchive::new(Cursor::new(FIXTURE))
        .unwrap()
        .by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let mut manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
    manifest["version"] = version.into();
    changed(
        "manifest.json",
        Some(&serde_json::to_vec(&manifest).unwrap()),
    )
}
#[test]
fn printable_ascii_versions_up_to_64_bytes_parse() {
    for version in ["1.0.0", "v2.3.4-rc.1+build.7", &"9".repeat(64)] {
        assert_eq!(
            Bundle::parse(&with_version(version))
                .unwrap()
                .manifest
                .version,
            version
        );
    }
}
#[test]
fn long_or_unprintable_versions_are_refused() {
    for version in [
        String::new(),
        "9".repeat(65),
        "1.0.0\n2.0.0".into(),
        "1.0.0 2.0.0".into(),
        "1.0.0\u{2028}".into(),
        "1.0.0\u{202E}".into(),
        "1.0.0\u{7F}".into(),
        "1.0.0é".into(),
    ] {
        assert!(
            Bundle::parse(&with_version(&version))
                .unwrap_err()
                .contains("version"),
            "{version:?}"
        );
    }
}
#[test]
fn only_a_radio_bundle_carries_the_radio_mark() {
    assert!(!Bundle::parse(FIXTURE).unwrap().manifest.radio);
    let radio = Bundle::parse(include_bytes!("fixtures/touchdeck-test-radio.tdfw")).unwrap();
    assert!(radio.manifest.radio);
    assert_eq!(radio.manifest.version, "test-only-radio");
}
