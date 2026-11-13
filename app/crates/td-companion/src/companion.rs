//! In-process seam for the native UI and behavioural tests.
//!
//! Keep a [`Companion`] for the program's lifetime, independently of windows. Clone its
//! [`Handle`] into each window. Dropping a handle or an observation never stops background
//! work; `request_shutdown`, `shutdown`, or dropping the owner does.
//!
//! Subscribe with `observe` before reading `snapshot` so a change between those steps
//! cannot be missed. Snapshots read each area independently; queued changes can overlap
//! the initial snapshot. Use changes to refresh the affected area or reread the snapshot,
//! rather than treating them as a transaction log. Apply received changes in order.
//! On `RecvError::Lagged`, resubscribe and take a fresh snapshot. Observation is bounded
//! (1024 changes), has no replay, and never waits for a UI to consume changes.
//! Operations use typed area handles, with no listener, authentication or JSON frames.
//! Start and shutdown require a Tokio runtime. Shutdown waits for owned async work, Actions
//! approval cleanup, in-flight Brandfetch refreshes and Device cleanup. It signals Integration,
//! Live tile and photo workers to finish. Keep blocking area operations off the UI thread.
//!
//! ```no_run
//! # async fn native_host(data: std::path::PathBuf) -> std::io::Result<()> {
//! use td_companion::companion::{Companion, Config};
//! let companion = Companion::start(Config::real(data)).await?;
//! let window = companion.handle();
//! let changes = window.observe();
//! let initial = window.snapshot();
//! // The window reads `initial`, then refreshes state as `changes` arrive.
//! drop((window, changes, initial)); // Devices continue running with no window.
//! companion.shutdown().await; // The program, rather than a window, owns shutdown.
//! # Ok(())
//! # }
//! ```
pub use crate::events::Change;
use crate::{clock::Clock, events::Events};
use std::{path::PathBuf, sync::Arc, time::Duration};
pub use tokio::sync::broadcast::error::RecvError;

/// Construction inputs that survive deletion of the local web adapter.
pub struct Config {
    pub data: PathBuf,
    pub devices: crate::devices::Config,
    pub mock: bool,
    pub asset_store_bytes: u64,
}
impl Config {
    pub fn real(data: PathBuf) -> Self {
        Self {
            devices: crate::devices::Config::real(&data),
            data,
            mock: false,
            asset_store_bytes: crate::store::ASSET_STORE_BYTES,
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn mock(data: PathBuf) -> Self {
        Self {
            devices: crate::devices::Config::mock(Some(data.join("devices.json"))),
            data,
            mock: true,
            asset_store_bytes: crate::store::ASSET_STORE_BYTES,
        }
    }
}

/// A typed view of the state sent to every new UI.
#[derive(Clone, serde::Serialize)]
pub struct Snapshot {
    pub devices: Vec<crate::devices::DeviceSummary>,
    pub new_boards: Vec<crate::devices::NewBoard>,
    pub integrations: Vec<crate::integrations::IntegrationSummary>,
    pub jobs: Vec<crate::jobs::Job>,
    pub approvals: Vec<crate::actions::ApprovalRequest>,
}

/// A window-independent handle. Construction and worker ownership stay in `Companion`.
#[derive(Clone)]
pub struct Handle {
    editor_root: PathBuf,
    pub(crate) editor_gif_sources:
        Arc<parking_lot::Mutex<std::collections::BTreeMap<String, crate::editor::GifSource>>>,
    pub(crate) actions: crate::actions::Actions,
    pub(crate) integrations: crate::integrations::Service,
    pub(crate) desktop: Arc<crate::tray::Desktop>,
    pub(crate) devices: crate::devices::DeviceManager,
    device_operations: crate::devices::Operations,
    pub(crate) firmware: crate::features::firmware::Firmware,
    pub(crate) events: Events,
    pub(crate) jobs: crate::jobs::Jobs,
    pub(crate) links: crate::link::Links,
    pub(crate) authority: crate::layouts::LayoutAuthority,
    pub(crate) live: crate::live::Live,
    pub(crate) photos: crate::photos::PhotoFrames,
    pub(crate) layouts: crate::store::LayoutStore,
    pub(crate) assets: crate::store::AssetStore,
    pub(crate) catalog: Arc<td_host::catalog::Catalog>,
    pub(crate) packs: Arc<td_host::packs::Packs>,
    pub(crate) brandfetch: crate::features::brandfetch::Brandfetch,
    pub(crate) clock: Clock,
    pub(crate) shutdown: tokio::sync::watch::Sender<bool>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) mock: bool,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) checkpoints: crate::checkpoints::Checkpoints,
}
impl Handle {
    pub(crate) fn new(
        mock: bool,
        data: &std::path::Path,
        asset_store_bytes: u64,
        grants: Arc<dyn crate::actions::GrantStore>,
    ) -> Self {
        let events = Events::default();
        let clock = Clock::default();
        let layouts = crate::store::LayoutStore::new(data.join("layouts"));
        let devices = crate::devices::DeviceManager::new(events.clone());
        let links = crate::link::Links::new(devices.clone());
        let actions = crate::actions::Actions::new(
            events.clone(),
            clock.clone(),
            devices.clone(),
            links.clone(),
            grants,
            mock,
        );
        let integrations = crate::integrations::Service::new(mock, events.clone());
        integrations.command_approval(Arc::new(actions.clone()));
        let settable = integrations.clone();
        let settable: crate::controls::Settable = Arc::new(move |key| settable.settable(key));
        let authority = crate::layouts::LayoutAuthority::new(
            layouts.pushed(),
            settable.clone(),
            devices.clone(),
        );
        let live = crate::live::Live::start(
            devices.clone(),
            links.clone(),
            integrations.clone(),
            authority.clone(),
        );
        let turned_on = integrations.clone();
        actions.attach(
            authority.clone(),
            integrations.controls.clone(),
            settable,
            Arc::new(move |id| turned_on.enabled(id)),
        );
        let jobs = crate::jobs::Jobs::new(events.clone(), clock.clone());
        let firmware = crate::features::firmware::Firmware::new(
            mock,
            devices.clone(),
            links.clone(),
            jobs.clone(),
            clock.clone(),
        );
        let photos = crate::photos::PhotoFrames::new(
            Some(data.join("photo-frame.json")),
            devices.clone(),
            links.clone(),
        );
        #[cfg(any(test, feature = "test-hooks"))]
        let checkpoints = crate::checkpoints::Checkpoints::default();
        let device_operations = crate::devices::Operations::new(
            devices.clone(),
            authority.clone(),
            photos.clone(),
            #[cfg(any(test, feature = "test-hooks"))]
            checkpoints.clone(),
        );
        Self {
            editor_root: data.to_path_buf(),
            editor_gif_sources: Default::default(),
            actions,
            integrations,
            live,
            authority,
            firmware,
            desktop: Arc::new(crate::tray::Desktop::new(mock)),
            jobs,
            photos,
            device_operations,
            links,
            assets: crate::store::AssetStore::new(
                data.join("assets"),
                layouts.clone(),
                clock.clone(),
                asset_store_bytes,
            ),
            layouts,
            catalog: Arc::new(td_host::catalog::Catalog::new(
                data.join("cache/catalog"),
                Box::new(td_host::catalog::HttpFetcher),
            )),
            packs: Arc::new(td_host::packs::Packs::new(
                data.join("packs"),
                Box::new(td_host::catalog::HttpDownload::new(
                    td_host::packs::MAX_DOWNLOAD,
                    std::time::Duration::from_secs(120),
                )),
            )),
            brandfetch: crate::features::brandfetch::Brandfetch::new(data.join("brandfetch"), mock),
            devices,
            events,
            clock,
            shutdown: tokio::sync::watch::channel(false).0,
            stopped: Default::default(),
            mock,
            #[cfg(any(test, feature = "test-hooks"))]
            checkpoints,
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            devices: self.devices.summaries(),
            new_boards: self.devices.new_boards(),
            integrations: self.integrations.summaries(),
            jobs: self.jobs.list(),
            approvals: self.actions.pending(),
        }
    }
    pub fn observe(&self) -> tokio::sync::broadcast::Receiver<Change> {
        self.events.subscribe()
    }
    pub fn devices(&self) -> &crate::devices::DeviceManager {
        &self.devices
    }
    pub fn open_editor(
        &self,
        device: &str,
    ) -> Result<crate::editor::Session, crate::editor::EditorError> {
        crate::editor::Session::open(self.clone(), self.editor_root.clone(), device)
    }
    pub fn layout_operations(&self) -> crate::layouts::Operations {
        crate::layouts::Operations::new(self.clone())
    }
    pub fn device_operations(&self) -> &crate::devices::Operations {
        &self.device_operations
    }
    pub fn integrations(&self) -> &crate::integrations::Service {
        &self.integrations
    }
    pub fn actions(&self) -> &crate::actions::Actions {
        &self.actions
    }
    pub fn firmware(&self) -> &crate::features::firmware::Firmware {
        &self.firmware
    }
    pub fn jobs(&self) -> &crate::jobs::Jobs {
        &self.jobs
    }
    pub fn request_shutdown(&self) {
        self.shutdown.send_replace(true);
    }
    pub fn shutdown_requested(&self) -> bool {
        *self.shutdown.borrow()
    }
    /// Waits for a shutdown request; the owner finishes worker cleanup in `shutdown`.
    pub async fn stopping(&self) {
        let _ = self.shutdown.subscribe().wait_for(|stop| *stop).await;
    }
    fn stop_workers(&self) {
        if !self.stopped.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.actions.stop();
            self.devices.stop();
            self.live.stop();
            self.photos.stop();
            self.integrations.stop();
            self.events.publish(&Change::Stopping);
        }
    }
    pub(crate) async fn maintain(&self) {
        let packs = self.packs.clone();
        let _ = tokio::task::spawn_blocking(move || packs.evict_idle()).await;
    }
}

/// How long `Companion::shutdown` waits for Device threads to close links and release keys.
const DEVICE_STOP_LIMIT: Duration = Duration::from_secs(5);

/// Owns background work, independently of any UI or local web connection.
pub struct Companion {
    handle: Handle,
    maintenance: tokio::task::JoinHandle<()>,
    brandfetch: tokio::task::JoinHandle<()>,
}
impl Companion {
    pub async fn start(config: Config) -> std::io::Result<Self> {
        std::fs::create_dir_all(&config.data)?;
        let handle = tokio::task::spawn_blocking(move || {
            let grants = Arc::new(crate::actions::FileGrants::new(
                config.data.join("allowed-actions.json"),
            ));
            (
                Handle::new(config.mock, &config.data, config.asset_store_bytes, grants),
                config.devices,
            )
        })
        .await
        .map_err(std::io::Error::other)?;
        Ok(Self::launch(handle.0, handle.1))
    }
    pub(crate) fn launch(handle: Handle, devices: crate::devices::Config) -> Self {
        handle.devices.start(devices);
        let work = handle.clone();
        let maintenance = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = work.stopping() => break,
                    _ = tokio::time::sleep(Duration::from_secs(60)) => work.maintain().await,
                }
            }
            work.stop_workers();
        });
        let work = handle.clone();
        let brandfetch = tokio::spawn(crate::features::brandfetch::refresh_loop(
            work,
            crate::features::brandfetch::REFRESH_TICK,
        ));
        Self {
            handle,
            maintenance,
            brandfetch,
        }
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    /// Requests shutdown and waits for maintenance, any in-flight Brandfetch refresh (including
    /// Asset and provenance writes), the Actions approval worker, and the Device threads (for at
    /// most `DEVICE_STOP_LIMIT`). On return, approvals are expired, no more actions can start and
    /// no Device key report types on the PC. If every Device thread finished within the limit,
    /// Device links are also closed, forwarders have finished and held keys are released. Already-started commands may keep
    /// running. Integration, Live tile and photo workers are signalled but are not joined.
    /// Blocking cleanup runs off the async runtime; `Drop` only requests and signals shutdown.
    pub async fn shutdown(mut self) {
        self.handle.request_shutdown();
        self.handle.stop_workers();
        let _ = (&mut self.maintenance).await;
        let _ = (&mut self.brandfetch).await;
        let handle = self.handle.clone();
        let _ = tokio::task::spawn_blocking(move || {
            handle.actions.wait_stopped();
            // Bounded: a Device thread stuck in kernel I/O must not keep the app from exiting.
            handle.devices.wait_stopped(DEVICE_STOP_LIMIT);
        })
        .await;
    }
}
impl Drop for Companion {
    fn drop(&mut self) {
        self.handle.request_shutdown();
        self.handle.stop_workers();
        self.maintenance.abort();
        self.brandfetch.abort();
    }
}
