use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub project: String,
    pub version: String,
    pub elf_sha256: String,
    pub files: Vec<FileEntry>,
    /// Only the opt-in radio build's bundle carries `"radio": true`. A missing mark is the normal build.
    #[serde(default)]
    pub radio: bool,
}
#[derive(Debug, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub offset: String,
    pub bytes: usize,
    pub sha256: String,
}
#[derive(Debug)]
pub struct Image {
    pub offset: u32,
    pub data: Vec<u8>,
}
#[derive(Debug)]
pub struct Bundle {
    pub manifest: Manifest,
    pub images: Vec<Image>,
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Bundle {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        if zip.len() != 4 {
            return Err("bundle must contain exactly the manifest and three firmware files".into());
        }
        let read = |zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
                    name: &str,
                    max: u64|
         -> Result<Vec<u8>, String> {
            let file = zip.by_name(name).map_err(|_| format!("missing {name}"))?;
            if file.size() > max {
                return Err(format!("{name} is too large"));
            }
            let mut data = Vec::new();
            file.take(max + 1)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
            if data.len() as u64 > max {
                return Err(format!("{name} is too large"));
            }
            Ok(data)
        };
        let manifest: Manifest = serde_json::from_slice(&read(&mut zip, "manifest.json", 16384)?)
            .map_err(|e| e.to_string())?;
        if manifest.format != 1 || manifest.project != "touchdeck" || manifest.files.len() != 3 {
            return Err("unsupported TouchDeck bundle".into());
        }
        if !(1..=64).contains(&manifest.version.len())
            || !manifest.version.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err("manifest version must be 1 to 64 printable ASCII characters".into());
        }
        let mut images = Vec::new();
        for (name, offset, max) in [
            ("bootloader.bin", 0, 0x8000),
            ("partition-table.bin", 0x8000, 0x1000),
            ("touchdeck.bin", 0x40000, 0x400000),
        ] {
            let entries: Vec<_> = manifest.files.iter().filter(|f| f.name == name).collect();
            if entries.len() != 1 {
                return Err(format!("manifest needs exactly one {name}"));
            }
            let entry = entries[0];
            let address = u32::from_str_radix(
                entry.offset.strip_prefix("0x").ok_or("offset needs 0x")?,
                16,
            )
            .map_err(|e| e.to_string())?;
            if address != offset {
                return Err(format!("unsafe offset for {name}"));
            }
            let data = read(&mut zip, name, max)?;
            if data.is_empty() || data.len() != entry.bytes || sha256(&data) != entry.sha256 {
                return Err(format!("{name} size or sha256 mismatch"));
            }
            images.push(Image {
                offset: address,
                data,
            });
        }
        let app = &images[2].data;
        if app.len() < 288
            || app[0] != 0xe9
            || app[32..36] != 0xabcd5432u32.to_le_bytes()
            || app[80..112].split(|b| *b == 0).next() != Some(b"touchdeck")
            || app[176..208]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
                != manifest.elf_sha256
        {
            return Err("application descriptor does not match TouchDeck manifest".into());
        }
        // Recovery must retain the known partition map, including storage and both OTA slots.
        let table = &images[1].data;
        for (name, kind, subtype, offset, size) in [
            ("otadata", 1, 0, 0xf000, 0x2000),
            ("ota_0", 0, 0x10, 0x40000, 0x400000),
            ("ota_1", 0, 0x11, 0x440000, 0x400000),
            ("storage", 1, 0x83, 0x840000, 0x7c0000),
        ] {
            if !table.as_chunks::<32>().0.iter().any(|p| {
                p[..2] == [0xaa, 0x50]
                    && p[2] == kind
                    && p[3] == subtype
                    && p[4..8] == u32::to_le_bytes(offset)
                    && p[8..12] == u32::to_le_bytes(size)
                    && p[12..28].split(|b| *b == 0).next() == Some(name.as_bytes())
            }) {
                return Err(format!("unsafe partition table: {name}"));
            }
        }
        Ok(Self { manifest, images })
    }
    pub fn app(&self) -> &[u8] {
        &self.images[2].data
    }
}
