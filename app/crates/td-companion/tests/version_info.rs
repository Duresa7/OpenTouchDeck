//! The version resource `build.rs` links into `td-companion.exe`, read back block by block the way Windows walks
//! it. CI reads the real file's version from the built program on a Windows runner; this runs on every OS.
#[path = "../version_info.rs"]
mod version_info;

use version_info::{CODE_PAGE, LANGUAGE, VersionInfo};

struct Block<'a> {
    key: String,
    text: bool,
    value: &'a [u8],
    children: Vec<Block<'a>>,
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16(&units).unwrap()
}

/// Reads the block that starts at `start`, an offset from the start of the resource. Returns it and its end.
fn read(resource: &[u8], start: usize) -> (Block<'_>, usize) {
    assert_eq!(start % 4, 0, "a block starts on a 4-byte boundary");
    let end = start + usize::from(u16_at(resource, start));
    let value_len = usize::from(u16_at(resource, start + 2));
    let text = match u16_at(resource, start + 4) {
        0 => false,
        1 => true,
        other => panic!("wType {other}"),
    };
    let mut at = start + 6;
    let key_start = at;
    while u16_at(resource, at) != 0 {
        at += 2;
    }
    let key = utf16(&resource[key_start..at]);
    at = (at + 2).next_multiple_of(4);
    let value_bytes = if text { value_len * 2 } else { value_len };
    let value = &resource[at..at + value_bytes];
    at += value_bytes;
    let mut children = Vec::new();
    while at.next_multiple_of(4) < end {
        let (child, child_end) = read(resource, at.next_multiple_of(4));
        children.push(child);
        at = child_end;
    }
    assert_eq!(at, end, "{key}: wLength ends with the last value or child");
    (
        Block {
            key,
            text,
            value,
            children,
        },
        end,
    )
}

const STRINGS: &[(&str, &str)] = &[
    ("CompanyName", "Duresa Kadi"),
    ("FileDescription", "Open TouchDeck"),
    ("FileVersion", "1.22.333"),
    ("OriginalFilename", "td-companion.exe"),
    // An odd and an even length, an empty value and text outside ASCII all have to keep the alignment.
    ("A", "é"),
    ("Empty", ""),
];

fn info() -> VersionInfo<'static> {
    VersionInfo {
        version: [1, 22, 333, 0],
        strings: STRINGS,
    }
}

#[test]
fn the_resource_carries_the_version_and_every_string() {
    let resource = info().resource();
    let (root, end) = read(&resource, 0);
    assert_eq!(end, resource.len());
    assert_eq!(root.key, "VS_VERSION_INFO");
    assert!(!root.text);

    // VS_FIXEDFILEINFO: the signature, then the file and product versions as two 32-bit halves each.
    assert_eq!(root.value.len(), 52);
    assert_eq!(u32_at(root.value, 0), 0xfeef_04bd);
    for version in [8, 16] {
        assert_eq!(u32_at(root.value, version), 1 << 16 | 22);
        assert_eq!(u32_at(root.value, version + 4), 333 << 16);
    }

    let [strings, var] = root.children.as_slice() else {
        panic!("the root holds StringFileInfo and VarFileInfo");
    };
    assert_eq!(strings.key, "StringFileInfo");
    let [table] = strings.children.as_slice() else {
        panic!("one string table");
    };
    assert_eq!(table.key, "040904B0");
    let read_back: Vec<(String, String)> = table
        .children
        .iter()
        .map(|string| {
            assert!(string.text && string.children.is_empty());
            let value = utf16(string.value);
            (
                string.key.clone(),
                value.strip_suffix('\0').unwrap().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = STRINGS
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(read_back, expected);

    assert_eq!(var.key, "VarFileInfo");
    let [translation] = var.children.as_slice() else {
        panic!("one translation");
    };
    assert_eq!(translation.key, "Translation");
    assert!(!translation.text);
    assert_eq!(
        translation.value,
        [LANGUAGE.to_le_bytes(), CODE_PAGE.to_le_bytes()].concat()
    );
}

#[test]
fn the_res_file_is_an_empty_entry_then_the_version_resource() {
    let resource = info().resource();
    let file = info().res_file();
    assert_eq!(file.len() % 4, 0);
    // The empty entry that opens a 32-bit .res file: no data, a 32-byte header, type 0 and name 0.
    assert_eq!(u32_at(&file, 0), 0);
    assert_eq!(u32_at(&file, 4), 32);
    assert_eq!(&file[8..16], [0xff, 0xff, 0, 0, 0xff, 0xff, 0, 0]);
    assert!(file[16..32].iter().all(|byte| *byte == 0));
    // RT_VERSION (16) named 1, in US English.
    assert_eq!(u32_at(&file, 32) as usize, resource.len());
    assert_eq!(u32_at(&file, 36), 32);
    assert_eq!(&file[40..48], [0xff, 0xff, 16, 0, 0xff, 0xff, 1, 0]);
    assert_eq!(u16_at(&file, 54), LANGUAGE);
    assert_eq!(&file[64..64 + resource.len()], resource);
    assert!(file[64 + resource.len()..].iter().all(|byte| *byte == 0));
}
