//! Open TouchDeck 2.0: the Companion and its native window in one program.
//!
//! Usage: `td-app`, or `td-app --mock-devices N` in a build with the `mock` feature.
//! `--packaged` marks an installed copy, as Start at sign-in runs it.
//! `TOUCHDECK_APP_DATA_DIR` moves the app data folder, as it does for the Companion.
//!
//! On Windows a notification-area icon keeps the app running after the window closes; its Open
//! item and left-click call `App::opener`, and Quit asks the Companion to stop. Elsewhere there
//! is no tray yet, so closing the window ends the program.
// A release build is a GUI program: Windows opens no console for it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
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

/// What the command line asks for.
#[derive(Debug, Default, PartialEq)]
struct Args {
    /// An installed copy started by the installer or at sign-in.
    packaged: bool,
    /// How many mock Devices to plug in, if any.
    mock_devices: Option<usize>,
}

fn parse(args: &[String]) -> Result<Args, Box<dyn std::error::Error>> {
    match args {
        [] => Ok(Args::default()),
        // Start at sign-in registers this command line (td-companion's startup.rs).
        [flag] if flag == "--packaged" => Ok(Args {
            packaged: true,
            mock_devices: None,
        }),
        #[cfg(feature = "mock")]
        [flag, count] if flag == "--mock-devices" => Ok(Args {
            packaged: false,
            mock_devices: Some(count.parse()?),
        }),
        _ => Err("unknown argument. Run td-app with no arguments.".into()),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Before any window or the tray, so both carry the app's identity.
    #[cfg(windows)]
    td_companion::tray::set_app_identity()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Args {
        packaged,
        mock_devices: mock,
    } = parse(&args)?;
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

    // Only Windows starts update checks. Blocking HTTP never runs on the UI/runtime thread.
    #[cfg(windows)]
    let update_worker = {
        let work = handle.clone();
        runtime.spawn(async move {
            let mut updater = td_updater::Updater::new(
                td_updater::GitHubClient::default(),
                td_updater::SystemClock::default(),
                semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("package version"),
            );
            loop {
                if work.shutdown_requested() {
                    break;
                }
                let updates = work.updates().clone();
                updater = match tokio::task::spawn_blocking(move || {
                    updates.poll(&mut updater, &td_core::app_dirs::Dirs::current());
                    updater
                })
                .await
                {
                    Ok(updater) => updater,
                    Err(_) => break,
                };
                tokio::select! {
                    _ = work.stopping() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
                }
            }
        })
    };

    // Ctrl+C asks the Companion to stop, which also closes the window.
    let interrupted = handle.clone();
    runtime.spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupted.request_shutdown();
        }
    });

    if packaged {
        // On by default the first time an installed copy runs, unless Windows already says off.
        let (first, folder) = (handle.clone(), data.clone());
        // `Runtime::spawn_blocking`, not `tokio::task::spawn_blocking`: the main thread is not
        // inside the runtime here, so the free function would panic.
        let initialized = runtime.block_on(runtime.spawn_blocking(move || {
            td_companion::tray::initialize_start_at_sign_in(&first, &folder)
        }))?;
        if let Err(error) = initialized {
            eprintln!("Open TouchDeck: could not set up Start at sign-in: {error}");
        }
    }

    // Kept until the Companion has shut down; dropping it removes the icon within 2 s.
    #[cfg(windows)]
    let mut tray = None;
    let shown = td_app::init_platform()
        .and_then(|()| {
            td_app::App::new(td_app::Options {
                data_dir: data,
                // The tray brings a hidden window back. Without one, closing the window ends
                // the program.
                close_to_tray: cfg!(windows),
            })
        })
        .and_then(|app| {
            #[cfg(windows)]
            {
                let opener = app.opener();
                tray = Some(td_companion::tray::start(
                    &handle,
                    std::sync::Arc::new(move || opener.open()),
                ));
            }
            app.run(&handle, runtime.handle())
        });
    runtime.block_on(companion.shutdown());
    #[cfg(windows)]
    update_worker.abort();
    #[cfg(windows)]
    drop(tray);
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    Ok(shown?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn no_arguments_and_the_start_at_sign_in_flag_are_accepted() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        assert_eq!(
            parse(&args(&["--packaged"])).unwrap(),
            Args {
                packaged: true,
                mock_devices: None
            }
        );
        assert!(parse(&args(&["--packaged", "extra"])).is_err());
        assert!(parse(&args(&["--open-in-browser"])).is_err());
    }
}
