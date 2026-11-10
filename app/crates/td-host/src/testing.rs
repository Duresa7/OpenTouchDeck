//! Test helpers shared by td-host's tests and td-companion's, behind the `testing` feature so
//! release builds never carry them.

use crate::{
    packs::{Curated, PackAuthor, PackLicense, REGISTRY, integrity_of},
    tarball::BLOCK,
};
use std::io::Write;

/// A gzipped ustar archive holding `files`, as npm publishes a package.
pub fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<_> = files
        .iter()
        .map(|(name, data)| (*name, b'0', *data))
        .collect();
    archive(&entries)
}

/// A gzipped ustar archive of `(name, type flag, data)` entries. A link entry (`b'2'`) points
/// at `/etc/passwd`.
pub fn archive(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    gzip(&tar(entries, true))
}

/// An uncompressed ustar archive of `(name, type flag, data)` entries, with or without its end
/// marker.
pub fn tar(entries: &[(&str, u8, &[u8])], end: bool) -> Vec<u8> {
    let mut tar = Vec::new();
    for (name, flag, data) in entries {
        let mut header = [0u8; BLOCK];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[156] = *flag;
        if *flag == b'2' {
            header[157..169].copy_from_slice(b"/etc/passwd\0");
        }
        header[257..263].copy_from_slice(b"ustar\0");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        tar.extend_from_slice(&header);
        tar.extend_from_slice(data);
        tar.resize(tar.len().div_ceil(BLOCK) * BLOCK, 0);
    }
    if end {
        tar.extend_from_slice(&[0; 2 * BLOCK]);
    }
    tar
}

pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

/// A curated listing for `prefix` that pins `tarball` on the npm registry: an MIT pack named
/// "Mono" with two icons. Override fields with struct update syntax.
pub fn listed(prefix: &str, tarball: &[u8]) -> Curated {
    Curated {
        prefix: prefix.into(),
        name: "Mono".into(),
        package: format!("@iconify-json/{prefix}"),
        version: "1.0.0".into(),
        tarball: format!("{REGISTRY}@iconify-json/{prefix}/-/{prefix}-1.0.0.tgz"),
        integrity: integrity_of(tarball),
        unpacked_size: 1000,
        icons: 2,
        license: PackLicense {
            title: "MIT".into(),
            spdx: Some("MIT".into()),
            url: None,
        },
        author: PackAuthor {
            name: "Someone".into(),
            url: None,
        },
        attribution: false,
        brands: false,
        palette: false,
        samples: vec!["spotify".into()],
    }
}
