//! Reads chosen files out of a gzipped tar archive, such as an npm tarball, without writing
//! anything. Every entry is checked before any file is returned: a link, or a path that could
//! leave the folder it would unpack into, refuses the whole archive on Linux and Windows alike.

use std::io::Read;

/// The most bytes an archive may unpack to.
pub const MAX_UNPACKED: u64 = 256 << 20;
/// The largest single file kept from an archive.
pub const MAX_FILE: u64 = 64 << 20;
/// The longest pax header or GNU long name the reader takes.
const MAX_HEADER: u64 = 64 << 10;
const BLOCK: usize = 512;

/// Whether an archive path stays inside the folder it would unpack into, on Linux and
/// Windows alike: relative, no `..`, no drive or stream (`:`), no NUL.
pub fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains([':', '\0'])
        && path.split(['/', '\\']).all(|part| part != "..")
}

/// Reads the regular files `want` picks, by their path in the archive. Later entries replace
/// earlier ones with the same path.
pub fn untar(gz: &[u8], want: impl Fn(&str) -> bool) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut reader = flate2::read::GzDecoder::new(gz).take(MAX_UNPACKED);
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut header = [0u8; BLOCK];
    // A pax header or GNU long name names the entry after it.
    let mut long_path: Option<String> = None;
    loop {
        let filled = fill(&mut reader, &mut header)?;
        if filled == 0 {
            return Err("the archive has no end marker".into());
        }
        if filled < BLOCK {
            return Err("the archive ends inside a header".into());
        }
        if header.iter().all(|b| *b == 0) {
            return Ok(out);
        }
        if !checksum_ok(&header) {
            return Err("the archive has a damaged header".into());
        }
        let text = |range: std::ops::Range<usize>| {
            let field = &header[range];
            let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
            String::from_utf8_lossy(&field[..end]).into_owned()
        };
        let size = octal(&header[124..136]).ok_or("the archive has a bad size field")?;
        let name = text(0..100);
        let prefix = if &header[257..262] == b"ustar" {
            text(345..500)
        } else {
            String::new()
        };
        let flag = header[156];
        let padded = size.div_ceil(BLOCK as u64) * BLOCK as u64;
        if matches!(flag, b'x' | b'L') {
            if size > MAX_HEADER {
                return Err("the archive has an oversized header".into());
            }
            let mut data = vec![0; padded as usize];
            reader
                .read_exact(&mut data)
                .map_err(|_| "the archive is cut short".to_owned())?;
            data.truncate(size as usize);
            if flag == b'L' {
                let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
                long_path = Some(String::from_utf8_lossy(&data[..end]).into_owned());
            } else if let Some(path) = pax_path(&data) {
                long_path = Some(path);
            }
            continue;
        }
        let path = match long_path.take() {
            Some(path) => path,
            None if prefix.is_empty() => name,
            None => format!("{prefix}/{name}"),
        };
        if !safe_path(&path) {
            return Err(format!("the archive has an unsafe path: {path}"));
        }
        if matches!(flag, b'1' | b'2' | b'K') {
            return Err(format!("the archive has a link: {path}"));
        }
        let regular = matches!(flag, b'0' | 0);
        if regular && want(&path) {
            if size > MAX_FILE {
                return Err(format!("{path} is too large"));
            }
            let mut data = vec![0; size as usize];
            reader
                .read_exact(&mut data)
                .map_err(|_| format!("{path} is cut short"))?;
            skip(&mut reader, padded - size)?;
            out.retain(|(p, _)| *p != path);
            out.push((path, data));
        } else {
            skip(&mut reader, padded)?;
        }
    }
}

/// Reads up to one block, returning how many bytes arrived before the end.
fn fill(reader: &mut impl Read, block: &mut [u8; BLOCK]) -> Result<usize, String> {
    let mut filled = 0;
    while filled < BLOCK {
        let n = reader
            .read(&mut block[filled..])
            .map_err(|e| format!("the archive is damaged: {e}"))?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

fn skip(reader: &mut impl Read, bytes: u64) -> Result<(), String> {
    let skipped = std::io::copy(&mut reader.take(bytes), &mut std::io::sink())
        .map_err(|e| format!("the archive is damaged: {e}"))?;
    if skipped < bytes {
        return Err("the archive is cut short".into());
    }
    Ok(())
}

/// An octal number field, padded with spaces or NULs.
fn octal(field: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(field).ok()?.trim_matches([' ', '\0']);
    u64::from_str_radix(text, 8).ok()
}

/// Whether the header's checksum matches: the sum of its bytes with the checksum field read
/// as spaces. Some writers summed signed bytes, so either sum is accepted.
fn checksum_ok(header: &[u8; BLOCK]) -> bool {
    let Some(stored) = octal(&header[148..156]) else {
        return false;
    };
    let field = 148..156;
    let (mut unsigned, mut signed) = (0u64, 0i64);
    for (i, byte) in header.iter().enumerate() {
        let byte = if field.contains(&i) { b' ' } else { *byte };
        unsigned += u64::from(byte);
        signed += i64::from(byte as i8);
    }
    stored == unsigned || i64::try_from(stored).is_ok_and(|stored| stored == signed)
}

/// The `path` record of a pax extended header, if it has one.
fn pax_path(data: &[u8]) -> Option<String> {
    let mut rest = data;
    let mut found = None;
    while !rest.is_empty() {
        let space = rest.iter().position(|b| *b == b' ')?;
        let len: usize = std::str::from_utf8(&rest[..space]).ok()?.parse().ok()?;
        if len <= space || len > rest.len() {
            return None;
        }
        let record = &rest[space + 1..len];
        let record = record.strip_suffix(b"\n").unwrap_or(record);
        if let Some(value) = record.strip_prefix(b"path=") {
            found = Some(String::from_utf8_lossy(value).into_owned());
        }
        rest = &rest[len..];
    }
    found
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A gzipped ustar archive holding `files`.
    pub fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
        let entries: Vec<_> = files
            .iter()
            .map(|(name, data)| (*name, b'0', *data))
            .collect();
        archive(&entries)
    }

    /// A gzipped ustar archive of `(name, type flag, data)` entries.
    pub fn archive(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
        gzip(&tar(entries, true))
    }

    fn tar(entries: &[(&str, u8, &[u8])], end: bool) -> Vec<u8> {
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

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(bytes).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn untar_keeps_only_wanted_files_and_refuses_damage() {
        let gz = tarball(&[
            ("package/icons.json", b"{}"),
            ("package/index.js", b"alert(1)"),
            ("package/LICENSE", b"MIT"),
        ]);
        let files = untar(&gz, |p| p != "package/index.js").unwrap();
        let names: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["package/icons.json", "package/LICENSE"]);
        assert_eq!(files[1].1, b"MIT");
        assert!(untar(&gz[..gz.len() / 2], |_| true).is_err());
        assert!(untar(b"not gzip", |_| true).is_err());
    }

    #[test]
    fn untar_refuses_a_bad_checksum_a_missing_end_or_short_padding() {
        let entries: &[(&str, u8, &[u8])] = &[("package/icons.json", b'0', b"{}")];
        let mut damaged = tar(entries, true);
        damaged[0] = b'q';
        let error = untar(&gzip(&damaged), |_| true).unwrap_err();
        assert!(error.contains("damaged header"), "{error}");

        let error = untar(&gzip(&tar(entries, false)), |_| true).unwrap_err();
        assert!(error.contains("no end marker"), "{error}");

        // The file's data is there, but the archive stops before its padding ends.
        let mut short = tar(entries, false);
        short.truncate(BLOCK + 2 + 10);
        let error = untar(&gzip(&short), |_| true).unwrap_err();
        assert!(error.contains("cut short"), "{error}");
    }

    #[test]
    fn untar_refuses_entries_that_could_leave_the_pack_folder() {
        let icons: &[u8] = b"{}";
        let refused = |entries: &[(&str, u8, &[u8])]| {
            let mut all = vec![("package/icons.json", b'0', icons)];
            all.extend_from_slice(entries);
            untar(&archive(&all), |_| true).unwrap_err()
        };
        for path in [
            "../icons.json",
            "package/../../icons.json",
            "package\\..\\..\\icons.json",
            "/etc/icons.json",
            "\\\\server\\share\\icons.json",
            "C:/icons.json",
            "C:icons.json",
            "package/icons.json:stream",
        ] {
            let error = refused(&[(path, b'0', icons)]);
            assert!(error.contains("unsafe path"), "{path}: {error}");
        }
        // Links are refused even when the reader would skip them.
        for flag in *b"12K" {
            let error = refused(&[("package/LICENSE", flag, b"")]);
            assert!(error.contains("link"), "{error}");
        }
        // A GNU long name or a pax path names the next entry, and is checked too.
        let long: &[u8] = b"package/../../evil.json\0";
        let error = refused(&[
            ("././@LongLink", b'L', long),
            ("package/ok.json", b'0', icons),
        ]);
        assert!(error.contains("../../evil.json"), "{error}");
        let pax: &[u8] = b"32 path=package/../../evil.json\n";
        let error = refused(&[("PaxHeader", b'x', pax), ("package/ok.json", b'0', icons)]);
        assert!(error.contains("../../evil.json"), "{error}");
        // A safe pax path is used as the entry's name.
        let pax: &[u8] = b"27 path=package/icons.json\n";
        let files = untar(
            &archive(&[("PaxHeader", b'x', pax), ("short", b'0', icons)]),
            |_| true,
        )
        .unwrap();
        assert_eq!(files[0].0, "package/icons.json");
    }
}
