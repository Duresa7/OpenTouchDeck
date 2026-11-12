//! Open TouchDeck 2.0: the Companion and its native window in one program.
//!
//! Usage: `td-app`, or `td-app --mock-devices N` in a build with the `mock` feature.
//! `TOUCHDECK_APP_DATA_DIR` moves the app data folder, as it does for the Companion.
use std::process::ExitCode;
use td_companion::companion::{Companion, Config};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Open TouchDeck: {error}");
            ExitCode::FAILURE
        }
    }
}

/// How many mock Devices to plug in, if any.
fn mock_devices(args: &[String]) -> Result<Option<usize>, Box<dyn std::error::Error>> {
    match args {
        [] => Ok(None),
        #[cfg(feature = "mock")]
        [flag, count] if flag == "--mock-devices" => Ok(Some(count.parse()?)),
        _ => Err("unknown argument. Run td-app with no arguments.".into()),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mock = mock_devices(&args)?;
    let data = td_core::app_dirs::data_dir().ok_or("no app data folder")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("companion")
        .build()?;

    let config = match mock {
        None => Config::real(data.clone()),
        #[cfg(feature = "mock")]
        Some(_) => {
            let mut config = Config::mock(data.clone());
            // Mock Devices stay in memory, never in a real devices.json.
            config.devices = td_companion::devices::Config::mock(None);
            config
        }
        #[cfg(not(feature = "mock"))]
        Some(_) => unreachable!("only mock builds take --mock-devices"),
    };
    let companion = runtime.block_on(Companion::start(config))?;
    let handle = companion.handle();
    #[cfg(feature = "mock")]
    if let (Some(count), Some((ports, _))) = (mock, handle.devices().mock()) {
        for n in 1..=count {
            ports.plug(
                &format!("mock{n:04}"),
                td_companion::devices::BoardKind::Device,
            );
        }
    }

    // Ctrl+C asks the Companion to stop, which also closes the window.
    let interrupted = handle.clone();
    runtime.spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupted.request_shutdown();
        }
    });

    let shown = td_app::init_platform()
        .and_then(|()| {
            td_app::App::new(td_app::Options {
                data_dir: data,
                // The tray rewrite starts the notification-area icon here, hands its "Open" item
                // `app.opener()` and its "Quit" item `handle.request_shutdown()`, and sets this
                // to true. Until a tray exists nothing could bring a hidden window back, so
                // closing it ends the program.
                close_to_tray: false,
            })
        })
        .and_then(|app| app.run(&handle, runtime.handle()));
    runtime.block_on(companion.shutdown());
    Ok(shown?)
}
