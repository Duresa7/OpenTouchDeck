use std::{env, fs, path::Path};

mod version_info;

/// The product name wherever Windows shows this program. The installer's product name must be the same text.
const PRODUCT_NAME: &str = "Open TouchDeck";
const COMPANY_NAME: &str = "Duresa Kadi";
const COPYRIGHT: &str = "Copyright Duresa Kadi";
const FILE_NAME: &str = "td-companion";

fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("app.rc", embed_resource::NONE)
            .manifest_required()
            .expect("embed the Windows app icon and Common Controls manifest");
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=app.manifest");
        embed_version_info();
        println!("cargo:rerun-if-changed=../../electron/installer/icon.ico");
    }
    embed_ui();
    // Each channel embeds its own bundle, and each bundle must be the build its channel names: the normal channel never
    // takes a bundle marked `"radio": true`, and the radio channel takes nothing else.
    embed_firmware("TOUCHDECK_FIRMWARE_BUNDLE", "bundled.tdfw", false);
    embed_firmware(
        "TOUCHDECK_RADIO_FIRMWARE_BUNDLE",
        "bundled-radio.tdfw",
        true,
    );
    for name in ["HEAD", "index"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=TD_BUILD={sha}");
}

/// Links the version resource into `td-companion.exe`: the product name, company, file description, original file
/// name and the Cargo version. Explorer, Task Manager and the Startup apps list show them, and a code-signing
/// service checks them before it signs.
fn embed_version_info() {
    // Only MSVC's linker takes a `.res` file as an input. The manifest arguments above are MSVC's too.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let part = |name: &str| -> u16 {
        env::var(name)
            .unwrap()
            .parse()
            .expect("each version part fits in 16 bits")
    };
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let info = version_info::VersionInfo {
        version: [
            part("CARGO_PKG_VERSION_MAJOR"),
            part("CARGO_PKG_VERSION_MINOR"),
            part("CARGO_PKG_VERSION_PATCH"),
            0,
        ],
        strings: &[
            ("CompanyName", COMPANY_NAME),
            ("FileDescription", PRODUCT_NAME),
            ("FileVersion", &version),
            ("InternalName", FILE_NAME),
            ("LegalCopyright", COPYRIGHT),
            ("OriginalFilename", &format!("{FILE_NAME}.exe")),
            ("ProductName", PRODUCT_NAME),
            ("ProductVersion", &version),
        ],
    };
    let path = Path::new(&env::var_os("OUT_DIR").unwrap()).join("version.res");
    fs::write(&path, info.res_file()).expect("write the version resource");
    // The program only: test executables need no version resource.
    println!("cargo:rustc-link-arg-bins={}", path.display());
}

fn embed_firmware(variable: &str, file: &str, radio: bool) {
    println!("cargo:rerun-if-env-changed={variable}");
    let bytes = match env::var_os(variable) {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", Path::new(&path).display());
            let bytes = fs::read(path).expect("read bundled Firmware");
            match td_link::bundle::Bundle::parse(&bytes) {
                Err(reason) => panic!("{variable} is not a valid Firmware bundle: {reason}"),
                Ok(bundle) if bundle.manifest.radio != radio => panic!(
                    "{variable} holds the {} build; it takes the {} build only",
                    if bundle.manifest.radio {
                        "radio"
                    } else {
                        "normal"
                    },
                    if radio { "radio" } else { "normal" }
                ),
                Ok(_) => bytes,
            }
        }
        None => Vec::new(),
    };
    // The release recipe names the SHA-256 it recorded in the release manifest, so the Companion embeds exactly
    // that file.
    let pinned = format!("{variable}_SHA256");
    println!("cargo:rerun-if-env-changed={pinned}");
    if let Some(want) = env::var_os(&pinned) {
        let got = td_link::bundle::sha256(&bytes);
        if bytes.is_empty() || !got.eq_ignore_ascii_case(&want.to_string_lossy()) {
            panic!("{variable} has SHA-256 {got}, not the {pinned} the release recorded");
        }
    }
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join(file),
        bytes,
    )
    .unwrap();
}

// Vite's default hash is eight URL-safe characters (including '-' and '_').
fn hashed(path: &str) -> bool {
    let stem = Path::new(path).file_stem().unwrap().to_string_lossy();
    let bytes = stem.as_bytes();
    bytes.len().checked_sub(9).is_some_and(|separator| {
        bytes[separator] == b'-'
            && bytes[separator + 1..]
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
    })
}
fn media_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}
fn collect(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).expect("read UI dist") {
        let entry = entry.expect("read UI file");
        let kind = entry.file_type().expect("read UI file type");
        if kind.is_dir() {
            collect(&entry.path(), files);
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
}
fn embed_ui() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let mut table = String::from("static UI_FILES: &[(&str, &str, bool, &[u8])] = &[\n");
    if dist.is_dir() {
        assert!(
            dist.join("index.html").is_file(),
            "UI dist must contain index.html"
        );
        let mut files = Vec::new();
        collect(&dist, &mut files);
        files.sort();
        for file in files {
            let key = file
                .strip_prefix(&dist)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let source = file.canonicalize().expect("resolve UI file");
            table.push_str(&format!(
                "({:?}, {:?}, {}, include_bytes!({:?})),\n",
                format!("/{key}"),
                media_type(&key),
                key != "index.html" && hashed(&key),
                source.to_str().unwrap()
            ));
        }
    } else {
        table.push_str("(\"/index.html\", \"text/html; charset=utf-8\", false, b\"<!doctype html><title>Open TouchDeck</title><p>UI not built. Run pnpm build:ui.</p>\"),\n");
    }
    table.push_str("];\n");
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("embedded_ui.rs"),
        table,
    )
    .expect("write embedded UI table");
}
