//! The bridge upgrade check's window on the user's data (`.github/workflows/bridge-upgrade.yml`). Not part of the app.
//!
//! `bridge_data seed-pairing-key <device>` saves a fixed test key for a Pairing in the OS keychain, as pairing does.
//! 1.x and 2.0 use the same `keyring` crate and entry names, so the check seeds the key with it before 1.x starts.
//!
//! `bridge_data report` starts the Companion on the app data folder td-app uses, through the same `td_core::app_dirs`
//! calls, and prints as JSON what 2.0 finds there: the Devices, the Layout last seen on each, the Pairings and whether
//! their keys load from the keychain, the CLIProxyAPI Integration with its secret, and the app's own settings.
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use td_companion::companion::{Companion, Config};
use td_companion::devices::{DeviceId, OsKeychain, PairingKeys};
use td_core::layout::Orientation;

/// The key the check seeds. Not secret: it is only ever in a test runner's keychain.
const TEST_KEY: [u8; 32] = *b"open-touchdeck-bridge-test-key!!";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, device] if command == "seed-pairing-key" => {
            let device = DeviceId::parse(device).ok_or("not a Device id")?;
            OsKeychain.save(&device, &TEST_KEY)?;
            Ok(())
        }
        [command] if command == "report" => {
            let report = tokio::runtime::Runtime::new()?.block_on(report())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => Err("usage: bridge_data seed-pairing-key <device> | bridge_data report".into()),
    }
}

async fn report() -> Result<Value, Box<dyn std::error::Error>> {
    let data = td_core::app_dirs::data_dir().ok_or("no app data folder")?;
    let companion = Companion::start(Config::real(data.clone())).await?;
    let handle = companion.handle();
    // The Device list loads on the Companion's own threads. Wait for it, or give up after a few seconds.
    let started = Instant::now();
    while handle.snapshot().devices.is_empty() && started.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let mut devices = Vec::new();
    for device in handle.snapshot().devices {
        let mut layouts = serde_json::Map::new();
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            if let Ok(seen) = handle
                .layout_operations()
                .last_seen(device.id.to_string(), orientation)
                .await
            {
                layouts.insert(
                    format!("{orientation:?}").to_lowercase(),
                    serde_json::to_value(&seen.layout)?,
                );
            }
        }
        devices
            .push(json!({ "id": device.id.to_string(), "name": device.name, "layouts": layouts }));
    }

    let pairings: Vec<Value> = handle
        .devices()
        .wifi_pairings()
        .into_iter()
        .map(|pairing| {
            let key = match OsKeychain.load(&pairing.device) {
                Ok(Some(key)) if key == TEST_KEY => "loads",
                Ok(Some(_)) => "different",
                Ok(None) => "missing",
                Err(_) => "keychain error",
            };
            json!({ "device": pairing.device.to_string(), "name": pairing.name, "key": key })
        })
        .collect();

    let cliproxyapi = handle
        .integrations()
        .view("cliproxyapi")
        .map(|view| json!({ "config": view.config, "secrets": view.secrets }))
        .unwrap_or(Value::Null);
    let dirs = td_core::app_dirs::Dirs::current();
    let settings = td_core::settings::AppSettings::load(&dirs).map(|settings| json!(settings));

    companion.shutdown().await;
    Ok(json!({
        "devices": devices,
        "pairings": pairings,
        "integrations": { "cliproxyapi": cliproxyapi },
        "app_settings": settings.unwrap_or_else(|error| json!({ "error": error.to_string() })),
        "startup_initialized": data.join("startup-initialized").exists(),
    }))
}
