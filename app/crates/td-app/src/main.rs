//! Open TouchDeck 2.0: the Companion and its native window in one program.
//!
//! Usage: `td-app`, or in a build with the `mock` feature `td-app --mock-devices N` or
//! `td-app --mock KIND,KIND…`, where a KIND is `device`, `radio` (the radio build, with two
//! paired apps), `v1` (Firmware too old to keep a name), `board` (a New board) or `fresh` (a New
//! board whose Firmware install starts at once, so setup opens). Mock builds accept every
//! Firmware install on their own. For screenshots, `TOUCHDECK_MOCK_PAGE` opens `setup`,
//! `settings` or `device:<id>` first, `TOUCHDECK_TEXT_SCALE` sets the text size, and
//! `TOUCHDECK_SNAPSHOT_DIR` saves the window to `<dir>/<name>` when `<dir>/request` names a file.
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
    /// The mock Devices and boards to plug in, if any.
    mock: Option<Vec<String>>,
}

fn parse(args: &[String]) -> Result<Args, Box<dyn std::error::Error>> {
    match args {
        [] => Ok(Args::default()),
        // Start at sign-in registers this command line (td-companion's startup.rs).
        [flag] if flag == "--packaged" => Ok(Args {
            packaged: true,
            mock: None,
        }),
        #[cfg(feature = "mock")]
        [flag, count] if flag == "--mock-devices" => Ok(Args {
            packaged: false,
            mock: Some(vec!["device".into(); count.parse()?]),
        }),
        #[cfg(feature = "mock")]
        [flag, kinds] if flag == "--mock" => {
            let kinds: Vec<String> = kinds.split(',').map(str::to_owned).collect();
            if let Some(unknown) = kinds
                .iter()
                .find(|k| !["device", "radio", "v1", "board", "fresh"].contains(&k.as_str()))
            {
                return Err(format!("unknown mock kind {unknown:?}").into());
            }
            Ok(Args {
                packaged: false,
                mock: Some(kinds),
            })
        }
        _ => Err("unknown argument. Run td-app with no arguments.".into()),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Before any window or the tray, so both carry the app's identity.
    #[cfg(windows)]
    td_companion::tray::set_app_identity()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Args { packaged, mock } = parse(&args)?;
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
    if let (Some(kinds), Some((ports, _))) = (&mock, handle.devices().mock()) {
        use td_companion::{devices::BoardKind, features::firmware::ConsentAnswer};
        handle.firmware().script(vec![ConsentAnswer::Accept; 1024]);
        for (n, kind) in kinds.iter().enumerate() {
            let serial = format!("mock{:04}", n + 1);
            let board = match kind.as_str() {
                "radio" => BoardKind::DeviceRadio,
                "v1" => BoardKind::DeviceV1,
                "board" | "fresh" => BoardKind::Silent,
                _ => BoardKind::Device,
            };
            if kind == "radio" {
                let deck = ports.board(&serial, board);
                deck.add_net_pairing([0x21; 16], "Laptop");
                deck.add_net_pairing([0x42; 16], "Studio PC");
            }
            ports.plug(&serial, board);
            if kind == "fresh" {
                let install = td_companion::features::firmware::FirmwareTask::new(
                    handle.clone(),
                    td_companion::features::firmware::Target::NewBoard(
                        td_companion::devices::DeviceId::parse(&serial).expect("a mock serial"),
                    ),
                );
                let devices = handle.devices().clone();
                let id = td_companion::devices::DeviceId::parse(&serial).expect("a mock serial");
                runtime.spawn(async move {
                    // Once the scan has found the board.
                    for _ in 0..100 {
                        if devices.new_boards().iter().any(|b| b.id == id) {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    install.start().await;
                });
            }
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
            // A download still blocked in a read when an earlier run hit the shutdown limit
            // leaves its staging folder behind. td-app takes no single-instance lock, so this
            // assumes no other copy is downloading an update right now.
            updater = match tokio::task::spawn_blocking(move || {
                updater.remove_stale_staging();
                updater
            })
            .await
            {
                Ok(updater) => updater,
                Err(_) => return,
            };
            loop {
                if work.shutdown_requested() {
                    break;
                }
                let poll_handle = work.clone();
                updater = match tokio::task::spawn_blocking(move || {
                    poll_handle.updates().poll(
                        &mut updater,
                        &td_core::app_dirs::Dirs::current(),
                        &|| poll_handle.shutdown_requested(),
                    );
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
                config: td_core::app_dirs::Dirs::current(),
                // The tray brings a hidden window back. Without one, closing the window ends
                // the program.
                close_to_tray: cfg!(windows),
            })
        })
        .and_then(|app| {
            #[cfg(feature = "mock")]
            if let Ok(page) = std::env::var("TOUCHDECK_MOCK_PAGE") {
                app.open_page(&page);
            }
            #[cfg(feature = "mock")]
            let _snapshots =
                std::env::var_os("TOUCHDECK_SNAPSHOT_DIR").map(|dir| app.snapshots(dir.into()));
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
    handle.request_shutdown();
    runtime.block_on(companion.shutdown());
    #[cfg(windows)]
    drop(tray);
    let shutdown_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    #[cfg(windows)]
    {
        let mut update_worker = update_worker;
        runtime.block_on(async {
            if tokio::time::timeout(
                shutdown_deadline.saturating_duration_since(std::time::Instant::now()),
                &mut update_worker,
            )
            .await
            .is_err()
            {
                update_worker.abort();
            }
        });
    }
    // Joining the cancelled poll and stopping the runtime share the existing limit.
    runtime
        .shutdown_timeout(shutdown_deadline.saturating_duration_since(std::time::Instant::now()));
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
                mock: None
            }
        );
        assert!(parse(&args(&["--packaged", "extra"])).is_err());
        assert!(parse(&args(&["--open-in-browser"])).is_err());
    }
}
