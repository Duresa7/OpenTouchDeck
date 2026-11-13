//! Typed Icon pack operations shared by native sessions and the legacy API.
use crate::companion::Handle;
use std::sync::Arc;
pub use td_host::packs::{PackError, PackInfo, PackStage};
use tokio::sync::watch;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Pack(#[from] PackError),
    #[error("Icon pack worker stopped")]
    Worker,
    #[error("App preferences could not be read or saved")]
    Settings(#[source] std::io::Error),
}
pub type Result<T> = std::result::Result<T, Arc<Error>>;

/// Obtained from Handle::icon_packs(). All disk and network work runs on a worker.
#[derive(Clone)]
pub struct Service(pub(crate) Handle);

impl Service {
    fn settings_dirs(&self) -> td_core::app_dirs::Dirs {
        if self.0.mock {
            td_core::app_dirs::Dirs {
                config: self.0.editor_root.join("config"),
            }
        } else {
            td_core::app_dirs::Dirs::current()
        }
    }
    pub async fn enabled(&self) -> Result<bool> {
        self.run(None, |s| s.read_enabled()).await
    }
    fn read_enabled(&self) -> std::result::Result<bool, Error> {
        td_core::settings::AppSettings::load(&self.settings_dirs())
            .map(|settings| settings.icon_packs_enabled)
            .map_err(Error::Settings)
    }
    pub async fn set_enabled(&self, on: bool) -> Result<()> {
        self.run(None, move |s| s.save_enabled(on)).await
    }
    fn save_enabled(&self, on: bool) -> std::result::Result<(), Error> {
        td_core::settings::AppSettings::set_icon_packs_enabled(&self.settings_dirs(), on)
            .map(|_| ())
            .map_err(Error::Settings)
    }

    pub(crate) fn start_install_job(
        &self,
        prefix: String,
    ) -> std::result::Result<crate::jobs::Job, crate::features::icon_packs::IconPackError> {
        use crate::{
            features::icon_packs::{IconPackError, InstallJob, InstallProgress},
            jobs::{JobScope, Stop},
        };
        if !self.0.packs.is_curated(&prefix) {
            return Err(PackError::NotFound.into());
        }
        let service = self.clone();
        Ok(self
            .0
            .jobs
            .start::<InstallJob>(JobScope::Companion, true, move |job| {
                let _gate = service.0.icon_packs_gate.blocking_lock();
                let result = service.install_blocking(
                    &prefix,
                    &|stage| job.progress(&InstallProgress { stage }),
                    &|| job.check().is_err(),
                );
                job.check()?;
                result.map_err(|error| Stop::from(IconPackError::from(error)))
            })?)
    }

    async fn run<T: Send + 'static>(
        &self,
        view: Option<watch::Sender<IconPacksView>>,
        work: impl FnOnce(&Self) -> std::result::Result<T, Error> + Send + 'static,
    ) -> Result<T> {
        let service = self.clone();
        let gate = service.0.icon_packs_gate.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            // Keep persistence and publication together, even if the caller drops its future.
            let _gate = gate;
            if let Some(view) = &view {
                view.send_modify(|v| {
                    v.busy = true;
                    v.error = None;
                    v.stage = None;
                });
            }
            let result = work(&service).map_err(Arc::new);
            if let Some(view) = &view {
                let packs = service.0.packs.list();
                view.send_modify(|v| {
                    v.packs = packs;
                    v.busy = false;
                    v.loaded = true;
                    v.error = result.as_ref().err().cloned();
                });
            }
            result
        })
        .await
        .map_err(|_| Arc::new(Error::Worker))?
    }

    pub async fn list(&self) -> Result<Vec<PackInfo>> {
        self.run(None, |s| Ok(s.0.packs.list())).await
    }

    pub async fn import(&self, bytes: Vec<u8>) -> Result<PackInfo> {
        self.run(None, move |s| Ok(s.0.packs.import(&bytes)?)).await
    }
    pub async fn remove(&self, prefix: &str) -> Result<()> {
        let prefix = prefix.to_owned();
        self.run(None, move |s| Ok(s.0.packs.remove(&prefix)?))
            .await
    }

    pub async fn install(&self, prefix: &str) -> Result<PackInfo> {
        let prefix = prefix.to_owned();
        self.run(None, move |s| {
            Ok(s.install_blocking(&prefix, &|_| {}, &|| false)?)
        })
        .await
    }

    /// Shared with the legacy Job, which already runs on a blocking worker.
    pub(crate) fn install_blocking(
        &self,
        prefix: &str,
        stage: &dyn Fn(PackStage),
        cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<PackInfo, PackError> {
        if self.0.shutdown_requested() || cancelled() {
            return Err(PackError::Cancelled);
        }
        self.0.packs.install(prefix, stage, &|| {
            self.0.shutdown_requested() || cancelled()
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct IconPacksView {
    pub packs: Vec<PackInfo>,
    /// None until preferences have been loaded successfully.
    pub enabled: Option<bool>,
    pub loaded: bool,
    pub busy: bool,
    pub stage: Option<PackStage>,
    pub error: Option<Arc<Error>>,
}

/// One page or picker. Call load on opening; subscribe before starting operations.
/// Awaited results identify success independently of the last displayed error.
/// Dropping an operation future does not cancel its worker or its final publication.
#[derive(Clone)]
pub struct IconPacksSession {
    service: Service,
    view: watch::Sender<IconPacksView>,
}
impl IconPacksSession {
    pub fn new(service: Service) -> Self {
        Self {
            service,
            view: watch::channel(IconPacksView::default()).0,
        }
    }
    pub fn view(&self) -> IconPacksView {
        self.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<IconPacksView> {
        self.view.subscribe()
    }
    pub async fn load(&self) -> Result<()> {
        let view = self.view.clone();
        self.service
            .run(Some(view.clone()), move |s| {
                let enabled = s.read_enabled()?;
                view.send_modify(|v| v.enabled = Some(enabled));
                Ok(())
            })
            .await
    }
    pub async fn remove(&self, prefix: &str) -> Result<()> {
        let prefix = prefix.to_owned();
        self.service
            .run(Some(self.view.clone()), move |s| {
                Ok(s.0.packs.remove(&prefix)?)
            })
            .await
    }
    pub async fn import(&self, bytes: Vec<u8>) -> Result<PackInfo> {
        self.service
            .run(Some(self.view.clone()), move |s| {
                Ok(s.0.packs.import(&bytes)?)
            })
            .await
    }
    pub async fn set_enabled(&self, on: bool) -> Result<()> {
        let view = self.view.clone();
        self.service
            .run(Some(view.clone()), move |s| {
                s.save_enabled(on)?;
                view.send_modify(|v| v.enabled = Some(on));
                Ok(())
            })
            .await
    }
    /// Retry a failed install by calling install again with the same prefix.
    pub async fn install(&self, prefix: &str) -> Result<PackInfo> {
        let prefix = prefix.to_owned();
        let view = self.view.clone();
        self.service
            .run(Some(view.clone()), move |s| {
                Ok(s.install_blocking(
                    &prefix,
                    &|stage| view.send_modify(|v| v.stage = Some(stage)),
                    &|| false,
                )?)
            })
            .await
    }
}
