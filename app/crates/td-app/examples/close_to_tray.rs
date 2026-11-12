//! The close-to-tray flow before the tray exists. Closing the window explains once that the app
//! keeps running, then hides it; pressing Enter in the terminal calls `Opener::open`, as the
//! tray's Open item will. Quit in the notice, or Ctrl+C, ends it. Runs on one mock Device in a
//! scratch data folder.
//!
//! ```sh
//! cargo run -p td-app --features mock --example close_to_tray
//! ```
use td_companion::{
    companion::{Companion, Config},
    devices::BoardKind,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::env::var_os("TOUCHDECK_APP_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("td-app-close-to-tray"));
    let runtime = tokio::runtime::Runtime::new()?;
    let mut config = Config::mock(data.clone());
    config.devices = td_companion::devices::Config::mock(None);
    let companion = runtime.block_on(Companion::start(config))?;
    let handle = companion.handle();
    if let Some((ports, _)) = handle.devices().mock() {
        ports.plug("mock0001", BoardKind::Device);
    }

    td_app::init_platform()?;
    let app = td_app::App::new(td_app::Options {
        data_dir: data.clone(),
        close_to_tray: true,
    })?;
    let opener = app.opener();
    std::thread::spawn(move || {
        for _ in std::io::stdin().lines() {
            opener.open();
        }
    });
    let interrupted = handle.clone();
    runtime.spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupted.request_shutdown();
        }
    });
    eprintln!(
        "Window state: {}. Press Enter to open the window again.",
        data.join(td_app::prefs::FILE).display()
    );
    let shown = app.run(&handle, runtime.handle());
    runtime.block_on(companion.shutdown());
    Ok(shown?)
}
