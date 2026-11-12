use std::{env, fs, path::Path};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
    }
    // Debug builds keep element ids and the tree data the headless UI tests search; release
    // builds leave them out.
    let debug = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("the Slint UI compiles");
    // The About licence test resolves this target's dependencies.
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    println!("cargo:rustc-env=TD_APP_TARGET={target}");
}

/// The Windows resources of every td-app executable: the app icon (Explorer, the taskbar and the
/// window), the tray icon, the Common Controls v6 manifest that `LoadIconMetric` needs, and the
/// version resource Explorer, Task Manager and Startup apps show. Icon 2 must match
/// `td_companion::tray::TRAY_ICON_RESOURCE`; icon 1 is the window's icon (`look::win`).
fn embed_windows_resources() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let icons = crate_dir.join("../../icons/app");
    let file = |path: &Path| {
        // Not canonicalize: on Windows it returns a `\\?\` path resource compilers reject.
        assert!(path.is_file(), "{} exists", path.display());
        println!("cargo:rerun-if-changed={}", path.display());
        // Every resource compiler takes forward slashes; a backslash would start an escape.
        path.display().to_string().replace('\\', "/")
    };
    let part = |name: &str| -> u16 {
        env::var(name)
            .unwrap()
            .parse()
            .expect("each version part fits in 16 bits")
    };
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let strings = [
        ("CompanyName", "Duresa Kadi"),
        ("FileDescription", "Open TouchDeck"),
        ("FileVersion", &version),
        ("InternalName", "td-app"),
        ("LegalCopyright", "Copyright Duresa Kadi"),
        ("OriginalFilename", "td-app.exe"),
        ("ProductName", "Open TouchDeck"),
        ("ProductVersion", &version),
    ]
    .map(|(name, value)| format!("      VALUE \"{name}\", \"{value}\""));
    // FILEOS 0x40004 is VOS_NT_WINDOWS32 and FILETYPE 1 is VFT_APP, as numbers so no header is
    // needed. 0x409 and 1200 are US English and Unicode.
    let rc = [
        format!("1 ICON \"{}\"", file(&icons.join("open-touchdeck.ico"))),
        format!("2 ICON \"{}\"", file(&icons.join("tray.ico"))),
        format!("1 24 \"{}\"", file(&crate_dir.join("app.manifest"))),
        "1 VERSIONINFO".into(),
        format!("FILEVERSION {major},{minor},{patch},0"),
        format!("PRODUCTVERSION {major},{minor},{patch},0"),
        "FILEOS 0x40004".into(),
        "FILETYPE 0x1".into(),
        "BEGIN".into(),
        "  BLOCK \"StringFileInfo\"".into(),
        "  BEGIN".into(),
        "    BLOCK \"040904b0\"".into(),
        "    BEGIN".into(),
        strings.join("\n"),
        "    END".into(),
        "  END".into(),
        "  BLOCK \"VarFileInfo\"".into(),
        "  BEGIN".into(),
        "    VALUE \"Translation\", 0x409, 1200".into(),
        "  END".into(),
        "END".into(),
    ]
    .join("\n")
        + "\n";
    let path = Path::new(&env::var_os("OUT_DIR").unwrap()).join("td-app.rc");
    fs::write(&path, rc).expect("write td-app.rc");
    // Every target: the tests and examples link td-companion's tray, which imports
    // LoadIconMetric, so they need the manifest too.
    embed_resource::compile_for_everything(&path, embed_resource::NONE)
        .manifest_required()
        .expect("embed td-app's Windows resources");
}
