#[cfg(feature = "test-hooks")]
use crate::devices::Config;
use crate::{api::Ctx, runtime::Runtime};
use std::{path::PathBuf, sync::Arc, time::Duration};
fn root() -> std::io::Result<PathBuf> {
    std::env::var_os("TOUCHDECK_APP_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|p| p.join(td_core::app_dirs::APP_ID)))
        .ok_or_else(|| std::io::Error::other("No app data directory."))
}
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let packaged = args == ["--packaged"];
    // How many mock Devices to plug in, when the Companion runs on mock ports.
    let mock_devices: Option<usize> = match args.as_slice() {
        [] | ["--packaged"] => None,
        #[cfg(feature = "test-hooks")]
        ["--test-real-device-console"] => None,
        #[cfg(feature = "test-hooks")]
        ["--emit-ts", path] => {
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, crate::bindings::generate())?;
            let test = std::path::Path::new(path).with_file_name("api.test.gen.ts");
            std::fs::write(test, crate::bindings::generate_test())?;
            return Ok(());
        }
        #[cfg(feature = "test-hooks")]
        ["--mock-devices", count] | ["--mock-devices", count, "--native-desktop"] => {
            Some(count.parse()?)
        }
        _ => return Err("Unknown Companion argument.".into()),
    };
    #[cfg(feature = "test-hooks")]
    if args == ["--test-real-device-console"]
        && std::env::var_os("TOUCHDECK_APP_DATA_DIR").is_none()
    {
        return Err(
            "--test-real-device-console needs TOUCHDECK_APP_DATA_DIR, so a test Companion never uses your real data folder."
                .into(),
        );
    }
    let root = root()?;
    std::fs::create_dir_all(&root)?;
    #[cfg(windows)]
    let dir = root.join("runtime");
    #[cfg(not(windows))]
    let dir = match std::env::var_os("TOUCHDECK_APP_DATA_DIR") {
        Some(_) => root.join("runtime"),
        None => std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or("XDG_RUNTIME_DIR is required")?
            .join("touchdeck"),
    };
    let runtime = Arc::new(Runtime::open(&dir)?);
    let old_port = std::fs::read(root.join("companion-settings.json")).ok();
    let listener = crate::serve::bind(&root).await?;
    assert!(listener.local_addr()?.ip().is_loopback());
    let port = listener.local_addr()?.port();
    let config = match mock_devices {
        None => crate::companion::Config::real(root.clone()),
        #[cfg(feature = "test-hooks")]
        Some(_) => {
            let mut config = crate::companion::Config::mock(root.clone());
            // Mock process Devices stay in memory, never in a real devices.json.
            config.devices = Config::mock(None);
            config
        }
        #[cfg(not(feature = "test-hooks"))]
        Some(_) => unreachable!("only test-hooks builds take --mock-devices"),
    };
    #[cfg(feature = "test-hooks")]
    let mock_ports = config.devices.mock.as_ref().map(|(ports, _)| ports.clone());
    let companion = crate::companion::Companion::start(config).await?;
    let mut handle = companion.handle();
    if args.contains(&"--native-desktop") {
        handle.desktop = Arc::new(crate::tray::Desktop::new(false));
    }
    #[allow(unused_mut)]
    let mut ctx = Ctx::new(
        handle,
        port,
        Some(runtime.clone()),
        crate::Limits::default(),
    );
    #[cfg(feature = "test-hooks")]
    {
        ctx.real_device_console = args == ["--test-real-device-console"];
    }
    let initial = ctx.clone();
    let written = tokio::task::spawn_blocking(move || {
        let value = initial.runtime_value();
        initial.runtime.as_ref().unwrap().repair(&value)
    })
    .await?;
    if !ctx.mock || args.contains(&"--native-desktop") {
        crate::tray::start(ctx.clone(), packaged);
    }
    if written.is_err() {
        ctx.desktop
            .notify("The runtime file failed its check. Restart Open TouchDeck.");
    }
    if old_port.is_some() && old_port != std::fs::read(root.join("companion-settings.json")).ok() {
        ctx.desktop
            .notify("The Companion port changed. Pair your browser again.");
    }
    written.map_err(|e| format!("The runtime file could not be secured: {e}"))?;
    if packaged {
        let marker = root.join("startup-initialized");
        if !marker.exists() {
            let startup = ctx.desktop.startup.clone();
            tokio::task::spawn_blocking(move || startup.set(true)).await??;
            std::fs::write(marker, b"1")?;
        }
    }
    #[cfg(feature = "test-hooks")]
    if let Some(count) = mock_devices {
        ctx.brandfetch.prefill_from_env();
        for n in 1..=count {
            mock_ports
                .as_ref()
                .expect("mock config")
                .plug(&format!("mock{n:04}"), crate::devices::BoardKind::Device);
        }
    }
    let repair = ctx.clone();
    let maintenance = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if !crate::api::maintain(&repair).await {
                let _ = repair.shutdown.send(true);
                break;
            }
        }
    });
    let signal = ctx.shutdown.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = signal.send(true);
        }
    });
    let stop = ctx.shutdown.subscribe();
    let limits = ctx.limits;
    crate::serve::serve(listener, crate::api::router(ctx), limits, stop).await;
    companion.shutdown().await;
    maintenance.abort();
    runtime.remove();
    Ok(())
}
