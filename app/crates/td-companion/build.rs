use std::{env, fs, path::Path};

fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Library tests also import LoadIconMetric, which requires Common Controls v6.
        embed_resource::compile_for_everything("app.rc", embed_resource::NONE)
            .manifest_required()
            .expect("embed the Windows app icon and Common Controls manifest");
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=icon.ico");
    }
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
