//! The Windows version resource for `td-companion.exe`, written as a compiled `.res` file that the MSVC linker
//! takes as an input. `build.rs` uses it, so the build needs no resource compiler and no extra crate.
//!
//! The layouts are [VS_VERSIONINFO](https://learn.microsoft.com/windows/win32/menurc/vs-versioninfo) and the
//! [RESOURCEHEADER](https://learn.microsoft.com/windows/win32/menurc/resourceheader) of a `.res` file.

/// What Windows shows for the file in Explorer's Details tab, Task Manager and the Startup apps list.
pub struct VersionInfo<'a> {
    /// The file and product version as `[major, minor, patch, build]`.
    pub version: [u16; 4],
    /// The string table, as name and value pairs such as `("ProductName", "Open TouchDeck")`.
    pub strings: &'a [(&'a str, &'a str)],
}

/// US English and Unicode: the one language the string table is written in.
pub const LANGUAGE: u16 = 0x0409;
pub const CODE_PAGE: u16 = 0x04b0;

const RT_VERSION: u16 = 16;
const VS_VERSION_INFO: u16 = 1;

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn pad4(bytes: &mut Vec<u8>) {
    bytes.resize(bytes.len().next_multiple_of(4), 0);
}

/// One block of the version tree: a six-byte header, a UTF-16 key, a value, then the child blocks. The value and
/// each child start on a 4-byte boundary. `value_len` counts UTF-16 units for text and bytes for binary data.
fn block(key: &str, text: bool, value: &[u8], value_len: usize, children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0; 6];
    out.extend(utf16z(key));
    pad4(&mut out);
    out.extend_from_slice(value);
    for child in children {
        pad4(&mut out);
        out.extend_from_slice(child);
    }
    let length = u16::try_from(out.len()).expect("a version block is under 64 KiB");
    let value_len = u16::try_from(value_len).expect("a version value is under 64 KiB");
    out[0..2].copy_from_slice(&length.to_le_bytes());
    out[2..4].copy_from_slice(&value_len.to_le_bytes());
    out[4..6].copy_from_slice(&u16::from(text).to_le_bytes());
    out
}

/// The 32-byte header of one entry in a `.res` file, for a resource whose type and name are numbers.
fn res_header(data_size: usize, kind: u16, name: u16, memory_flags: u16, language: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend((data_size as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    for id in [kind, name] {
        out.extend(0xffffu16.to_le_bytes());
        out.extend(id.to_le_bytes());
    }
    out.extend(0u32.to_le_bytes()); // DataVersion
    out.extend(memory_flags.to_le_bytes());
    out.extend(language.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // Version
    out.extend(0u32.to_le_bytes()); // Characteristics
    out
}

impl VersionInfo<'_> {
    /// The VS_VERSIONINFO resource data.
    pub fn resource(&self) -> Vec<u8> {
        let [major, minor, patch, build] = self.version;
        let high = u32::from(major) << 16 | u32::from(minor);
        let low = u32::from(patch) << 16 | u32::from(build);
        let fixed: Vec<u8> = [
            0xfeef_04bd, // dwSignature
            0x0001_0000, // dwStrucVersion
            high,        // dwFileVersionMS
            low,         // dwFileVersionLS
            high,        // dwProductVersionMS
            low,         // dwProductVersionLS
            0x3f,        // dwFileFlagsMask
            0,           // dwFileFlags
            0x0004_0004, // dwFileOS: VOS_NT_WINDOWS32
            1,           // dwFileType: VFT_APP
            0,           // dwFileSubtype
            0,           // dwFileDateMS
            0,           // dwFileDateLS
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
        let strings: Vec<Vec<u8>> = self
            .strings
            .iter()
            .map(|(name, value)| {
                let value = utf16z(value);
                block(name, true, &value, value.len() / 2, &[])
            })
            .collect();
        let table = block(
            &format!("{LANGUAGE:04X}{CODE_PAGE:04X}"),
            true,
            &[],
            0,
            &strings,
        );
        let translation: Vec<u8> = [LANGUAGE, CODE_PAGE]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        block(
            "VS_VERSION_INFO",
            false,
            &fixed,
            fixed.len(),
            &[
                block("StringFileInfo", true, &[], 0, &[table]),
                block(
                    "VarFileInfo",
                    true,
                    &[],
                    0,
                    &[block(
                        "Translation",
                        false,
                        &translation,
                        translation.len(),
                        &[],
                    )],
                ),
            ],
        )
    }

    /// A compiled resource file (`.res`) that holds only the version resource.
    pub fn res_file(&self) -> Vec<u8> {
        let data = self.resource();
        // Every 32-bit .res file starts with an empty entry.
        let mut out = res_header(0, 0, 0, 0, 0);
        // MOVEABLE | PURE, the flags the resource compiler gives a version resource.
        out.extend(res_header(
            data.len(),
            RT_VERSION,
            VS_VERSION_INFO,
            0x0030,
            LANGUAGE,
        ));
        out.extend(data);
        pad4(&mut out);
        out
    }
}
