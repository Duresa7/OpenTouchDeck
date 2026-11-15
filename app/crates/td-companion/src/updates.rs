//! Native updater state. Only the native Windows host starts polling.
use parking_lot::Mutex;
use std::sync::Arc;
use td_updater::{Clock, HttpClient, UpdateResult, Updater};

#[derive(Clone, Debug, Default, serde::Serialize)]
pub enum Status {
    #[default]
    Idle,
    UpToDate,
    Ready {
        version: String,
    },
    Failed {
        message: String,
    },
    Installing,
}

#[derive(Default)]
struct State {
    ready: Option<td_updater::UpdateReady>,
}

#[derive(Clone)]
pub struct Updates {
    state: Arc<Mutex<State>>,
    status: tokio::sync::watch::Sender<Status>,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            status: tokio::sync::watch::channel(Status::Idle).0,
        }
    }
}

impl Updates {
    pub fn snapshot(&self) -> Status {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Status> {
        self.status.subscribe()
    }

    /// Blocking worker operation. Load the preference anew before each poll.
    pub fn poll<H: HttpClient, C: Clock>(
        &self,
        updater: &mut Updater<H, C>,
        dirs: &td_core::app_dirs::Dirs,
        cancelled: &dyn Fn() -> bool,
    ) {
        if cancelled() {
            return;
        }
        let result = match td_core::settings::AppSettings::load(dirs) {
            Ok(settings) => updater.poll(&settings, cancelled),
            Err(error) => Some(UpdateResult::Failed(td_updater::Failure::Io(error))),
        };
        let Some(result) = result else { return };
        let mut state = self.state.lock();
        if cancelled() || matches!(self.snapshot(), Status::Installing) {
            return;
        }
        state.ready = None;
        let status = match result {
            UpdateResult::UpToDate => Status::UpToDate,
            UpdateResult::Failed(error) => Status::Failed {
                message: error.to_string(),
            },
            UpdateResult::UpdateReady(ready) => {
                let version = ready.version().to_string();
                state.ready = Some(ready);
                Status::Ready { version }
            }
        };
        self.status.send_replace(status);
    }

    /// Runs off the UI thread. The host's shutdown request closes the UI event loop,
    /// flushes editor Workers and then calls Companion::shutdown.
    #[cfg(windows)]
    pub fn install_now(
        &self,
        handle: &crate::companion::Handle,
    ) -> Result<(), td_updater::Failure> {
        let ready = {
            let mut state = self.state.lock();
            let Some(ready) = state.ready.take() else {
                return Ok(());
            };
            self.status.send_replace(Status::Installing);
            ready
        };
        let result: Result<(), td_updater::Failure> = (|| {
            let system = std::env::var_os("SystemRoot")
                .ok_or_else(|| std::io::Error::other("missing SystemRoot"))?;
            ready.install(
                &std::path::PathBuf::from(system).join("System32/msiexec.exe"),
                |command| {
                    require_unelevated()?;
                    command.spawn().map(|_| ())
                },
                || handle.request_shutdown(),
            )
        })();
        if let Err(ref error) = result {
            self.status.send_replace(Status::Failed {
                message: format!("Couldn't install the update: {error}. Keep using this version and try again after reopening Open TouchDeck."),
            });
        }
        result
    }
}

/// Command::spawn inherits our token. Refuse an elevated host rather than running
/// this per-user MSI as administrator. There is no ShellExecute/runas elevation.
#[cfg(windows)]
fn require_unelevated() -> std::io::Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // SAFETY: all out pointers refer to initialized local storage of the stated size.
    // The token is closed on both paths after a successful OpenProcessToken.
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut length = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        );
        let error = (ok == 0).then(std::io::Error::last_os_error);
        CloseHandle(token);
        if let Some(error) = error {
            return Err(error);
        }
        if elevation.TokenIsElevated != 0 {
            return Err(std::io::Error::other(
                "Close Open TouchDeck and reopen it without Run as administrator before installing an update",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Http(std::sync::atomic::AtomicUsize);
    impl HttpClient for Http {
        fn get(&self, _: &str, _: &str) -> Result<Box<dyn std::io::Read>, td_updater::Failure> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(std::io::Cursor::new(b"[]")))
        }
    }
    struct Time;
    impl Clock for Time {
        fn now(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
    }
    #[test]
    fn preference_is_reloaded_and_results_are_visible_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = td_core::app_dirs::Dirs {
            config: temp.path().to_owned(),
        };
        let updates = Updates::default();
        let mut changes = updates.subscribe();
        let mut updater = Updater::new(
            Http(Default::default()),
            Time,
            semver::Version::new(2, 0, 0),
        );
        td_core::settings::AppSettings {
            automatic_update_checks: false,
            ..Default::default()
        }
        .save(&dirs)
        .unwrap();
        updates.poll(&mut updater, &dirs, &|| false);
        assert!(matches!(updates.snapshot(), Status::Idle));
        assert!(!changes.has_changed().unwrap());
        td_core::settings::AppSettings::default()
            .save(&dirs)
            .unwrap();
        updates.poll(&mut updater, &dirs, &|| false);
        assert!(matches!(updates.snapshot(), Status::UpToDate));
        assert!(changes.has_changed().unwrap());
        assert!(matches!(*changes.borrow_and_update(), Status::UpToDate));
        std::fs::write(dirs.app_settings(), b"invalid").unwrap();
        updates.poll(&mut updater, &dirs, &|| false);
        assert!(matches!(updates.snapshot(), Status::Failed { .. }));
        assert!(changes.has_changed().unwrap());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn install_without_a_staged_update_does_not_quit_or_change_status() {
        let temp = tempfile::tempdir().unwrap();
        let companion = crate::companion::Companion::start(crate::companion::Config::mock(
            temp.path().to_owned(),
        ))
        .await
        .unwrap();
        let handle = companion.handle();
        handle.updates().install_now(&handle).unwrap();
        assert!(!handle.shutdown_requested());
        assert!(matches!(handle.updates().snapshot(), Status::Idle));
        companion.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_during_poll_does_not_publish_a_result() {
        use crate::companion::{Companion, Config};
        use std::sync::mpsc;

        struct BlockingHttp {
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        }
        impl HttpClient for BlockingHttp {
            fn get(&self, _: &str, _: &str) -> Result<Box<dyn std::io::Read>, td_updater::Failure> {
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
                Ok(Box::new(std::io::Cursor::new(b"[]")))
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let companion = Companion::start(Config::mock(temp.path().to_owned()))
            .await
            .unwrap();
        let handle = companion.handle();
        let poll_handle = handle.clone();
        let dirs = td_core::app_dirs::Dirs {
            config: temp.path().to_owned(),
        };
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let mut updater = Updater::new(
                BlockingHttp {
                    entered: entered_tx,
                    release: release_rx,
                },
                Time,
                semver::Version::new(2, 0, 0),
            );
            poll_handle
                .updates()
                .poll(&mut updater, &dirs, &|| poll_handle.shutdown_requested());
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        handle.request_shutdown();
        release_tx.send(()).unwrap();
        worker.await.unwrap();
        assert!(matches!(handle.updates().snapshot(), Status::Idle));
        companion.shutdown().await;
    }
}
