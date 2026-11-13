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
    status: Status,
    ready: Option<td_updater::UpdateReady>,
}

#[derive(Clone, Default)]
pub struct Updates(Arc<Mutex<State>>);
impl Updates {
    pub fn snapshot(&self) -> Status {
        self.0.lock().status.clone()
    }

    /// Blocking worker operation. Load the preference anew before each poll.
    pub fn poll<H: HttpClient, C: Clock>(
        &self,
        updater: &mut Updater<H, C>,
        dirs: &td_core::app_dirs::Dirs,
    ) {
        let result = match td_core::settings::AppSettings::load(dirs) {
            Ok(settings) => updater.poll(&settings),
            Err(error) => Some(UpdateResult::Failed(td_updater::Failure::Io(error))),
        };
        let Some(result) = result else { return };
        let mut state = self.0.lock();
        if matches!(state.status, Status::Installing) {
            return;
        }
        state.ready = None;
        state.status = match result {
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
    }

    /// Runs off the UI thread. Keep staging alive until Windows Installer finishes.
    #[cfg(windows)]
    pub fn install_now(
        &self,
        handle: &crate::companion::Handle,
    ) -> Result<(), td_updater::Failure> {
        let ready = {
            let mut state = self.0.lock();
            let ready = state.ready.take().ok_or(td_updater::Failure::Signature)?;
            state.status = Status::Installing;
            ready
        };
        let result = (|| {
            ready.verify()?;
            let system = std::env::var_os("SystemRoot")
                .ok_or_else(|| std::io::Error::other("missing SystemRoot"))?;
            let mut child = std::process::Command::new(
                std::path::PathBuf::from(system).join("System32/msiexec.exe"),
            )
            .arg("/i")
            .arg(ready.path())
            .arg("/passive")
            .arg("/norestart")
            .spawn()?;
            handle.request_shutdown();
            // The host exits through Companion::shutdown. This thread keeps the private MSI
            // alive while msiexec reads it; process exit leaves staging for OS cleanup.
            std::thread::spawn(move || {
                let _ = child.wait();
                drop(ready);
            });
            Ok(())
        })();
        if let Err(ref error) = result {
            self.0.lock().status = Status::Failed {
                message: error.to_string(),
            };
        }
        result
    }
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
        let mut updater = Updater::new(
            Http(Default::default()),
            Time,
            semver::Version::new(2, 0, 0),
        );
        td_core::settings::AppSettings {
            automatic_update_checks: false,
        }
        .save(&dirs)
        .unwrap();
        updates.poll(&mut updater, &dirs);
        assert!(matches!(updates.snapshot(), Status::Idle));
        td_core::settings::AppSettings::default()
            .save(&dirs)
            .unwrap();
        updates.poll(&mut updater, &dirs);
        assert!(matches!(updates.snapshot(), Status::UpToDate));
        std::fs::write(dirs.app_settings(), b"invalid").unwrap();
        updates.poll(&mut updater, &dirs);
        assert!(matches!(updates.snapshot(), Status::Failed { .. }));
    }
}
