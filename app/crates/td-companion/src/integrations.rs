//! Running Integration module shared by the Companion Handle and local web adapter.
//!
//! User changes, persistence, pending delivery reconciliation, summary caching and ordered
//! publication cross this interface. Polling does upstream I/O outside the registry lock.
//! Operations are synchronous; callers keep them off the UI and async runtime threads.
//! Command approval is supplied by the Companion, never answered through this module.
use crate::{events::Events, live::StreamKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use td_host::integrations::secrets::{MemorySecrets, OsSecrets};
use td_host::integrations::{Control, Delivery, Environment, LiveSample, Registry};
pub use td_host::integrations::{Error, IntegrationSummary, IntegrationView};
use ts_rs::TS;

/// The Integrations' Controls by Integration id, held outside the registry lock so a slow poll never holds them up.
pub type Controls = Arc<BTreeMap<String, Arc<dyn Control>>>;

#[derive(Clone)]
pub struct Service {
    registry: Arc<Mutex<Result<Registry, Error>>>,
    runtime: Arc<PollRuntime>,
    summaries: Arc<Mutex<Vec<IntegrationSummary>>>,
    pending: Arc<Pending>,
    events: Events,
    pub(crate) controls: Controls,
    #[cfg(any(test, feature = "test-hooks"))]
    media: Option<td_host::integrations::media::fake::FakeMedia>,
    #[cfg(any(test, feature = "test-hooks"))]
    volume: Option<td_host::integrations::volume::fake::FakeVolume>,
}
#[derive(Default)]
struct PollRuntime {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Drop for PollRuntime {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.handle.get_mut().unwrap().take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}
struct PollWorker {
    jobs: Option<std::sync::mpsc::SyncSender<td_host::integrations::Poll>>,
    results: std::sync::mpsc::Receiver<td_host::integrations::PollState>,
    handle: Option<std::thread::JoinHandle<()>>,
    busy: bool,
}
impl PollWorker {
    fn new(id: String) -> Self {
        let (jobs, work) = std::sync::mpsc::sync_channel::<td_host::integrations::Poll>(1);
        let (done, results) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            while let Ok(poll) = work.recv() {
                let state = std::panic::catch_unwind(std::panic::AssertUnwindSafe(poll))
                    .unwrap_or_else(|_| {
                        eprintln!("Companion: the {id} poll failed; it runs again when due.");
                        Box::new(())
                    });
                if done.send(state).is_err() {
                    break;
                }
            }
        });
        Self {
            jobs: Some(jobs),
            results,
            handle: Some(handle),
            busy: false,
        }
    }
}
impl Drop for PollWorker {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
type Pending = Mutex<HashMap<(String, String), Delivery>>;

/// One setup interaction, including its intent to enable after a successful save.
///
/// Keep this value with the setup window. Cancel or drop it to discard that intent;
/// a later save through another handle cannot complete a cancelled setup. No work is
/// queued by setup, and no secret is held here. A failed save can be corrected and retried.
pub struct Setup {
    service: Service,
    integration: String,
    enable_when_ready: bool,
}
impl Setup {
    /// Saves configuration first (which may change the secret's origin), then secrets,
    /// then enables if requested and ready. Poll results and other user writes cannot
    /// interleave these steps. Settings and secrets already saved before an error stay
    /// saved; enabling is always last. `enabled` in a form is ignored: setup owns intent.
    pub fn save(
        &mut self,
        mut config: Value,
        secrets: BTreeMap<String, String>,
    ) -> Result<IntegrationView, Error> {
        if let Some(fields) = config.as_object_mut() {
            fields.remove("enabled");
        }
        let view = self.service.with_registry(|registry| {
            let integration = registry.get(&self.integration)?;
            configure(integration.as_mut(), config)?;
            for (field, value) in secrets {
                integration.secret_put(&field, &value)?;
            }
            if self.enable_when_ready {
                if integration.summary().status.state == "not_configured" {
                    return Err(Error::ConfigInvalid);
                }
                integration.set_enabled(true)
            } else {
                integration.view()
            }
        })?;
        self.enable_when_ready = false;
        Ok(view)
    }

    /// Discards pending enable intent. Previously saved settings are not rolled back.
    pub fn cancel(self) {}
}

impl Service {
    pub fn list(&self) -> Result<Vec<IntegrationSummary>, Error> {
        self.with_registry(|registry| Ok(registry.list()))
    }

    pub fn view(&self, id: &str) -> Result<IntegrationView, Error> {
        self.with_registry(|registry| registry.get(id)?.view())
    }

    /// An omitted `enabled` preserves the stored switch under the same lock as the
    /// save. A stale settings form therefore cannot undo another client's switch.
    pub fn configure(&self, id: &str, config: Value) -> Result<IntegrationView, Error> {
        self.with_registry(|registry| configure(registry.get(id)?.as_mut(), config))
    }

    /// Changes only the switch, preserving configuration and secrets.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<IntegrationView, Error> {
        self.with_registry(|registry| registry.get(id)?.set_enabled(enabled))
    }

    pub fn secret_put(&self, id: &str, field: &str, value: &str) -> Result<(), Error> {
        self.with_registry(|registry| registry.get(id)?.secret_put(field, value))
    }

    pub fn secret_delete(&self, id: &str, field: &str) -> Result<(), Error> {
        self.with_registry(|registry| registry.get(id)?.secret_delete(field))
    }

    /// Requests polling without waiting for upstream I/O.
    pub fn refresh(&self, id: &str) -> Result<(), Error> {
        self.with_registry(|registry| {
            registry.get(id)?.request_refresh();
            Ok(())
        })
    }

    /// Starts setup without changing persisted state. The caller can close the setup
    /// window at any time; only a successful save can complete its enable intent.
    pub fn setup(&self, id: &str, enable_when_ready: bool) -> Result<Setup, Error> {
        self.view(id)?;
        Ok(Setup {
            service: self.clone(),
            integration: id.to_owned(),
            enable_when_ready,
        })
    }

    pub(crate) fn stop(&self) {
        self.runtime
            .stop
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.runtime.handle.lock().unwrap().as_ref() {
            handle.thread().unpark();
        }
    }

    pub(crate) fn command_approval(
        &self,
        approval: Arc<dyn td_host::integrations::CommandApproval>,
    ) {
        if let Ok(registry) = self.registry.lock().unwrap().as_mut()
            && let Ok(custom) = registry.get("custom")
        {
            custom.command_approval(approval);
        }
    }
    pub(crate) fn new(mock: bool, events: Events) -> Self {
        let media = mock.then(td_host::integrations::media::fake::FakeMedia::default);
        let volume = mock.then(td_host::integrations::volume::fake::FakeVolume::default);
        let registry = Registry::new(Environment {
            media_provider: media.as_ref().map(|fake| {
                Arc::new(fake.clone()) as Arc<dyn td_host::integrations::media::MediaProvider>
            }),
            volume: volume.as_ref().map(|fake| fake.factory()),
            secrets: if mock {
                Arc::new(MemorySecrets::default())
            } else {
                Arc::new(OsSecrets)
            },
            config_file: (!mock).then(|| td_core::app_dirs::Dirs::current().sources_config()),
            fetch: if mock {
                Arc::new(|| Box::new(FakeFetch))
            } else {
                Arc::new(|| Box::new(td_host::sources::HttpFetch::new()))
            },
        });
        let controls = Arc::new(
            registry
                .as_ref()
                .map(Registry::controls)
                .unwrap_or_default(),
        );
        let service = Self {
            registry: Arc::new(Mutex::new(registry)),
            runtime: Arc::default(),
            summaries: Arc::default(),
            pending: Arc::default(),
            events: events.clone(),
            controls,
            #[cfg(any(test, feature = "test-hooks"))]
            media,
            #[cfg(any(test, feature = "test-hooks"))]
            volume,
        };
        if let Ok(registry) = service.registry.lock().unwrap().as_ref() {
            *service.summaries.lock().unwrap() = registry.list();
        }
        let cache = service.summaries.clone();
        let pending = service.pending.clone();
        let state = service.registry.clone();
        let stop = service.runtime.stop.clone();
        let handle = std::thread::spawn(move || {
            let mut workers = BTreeMap::<String, PollWorker>::new();
            while !stop.load(std::sync::atomic::Ordering::Acquire) {
                std::thread::park_timeout(std::time::Duration::from_secs(1));
                if stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                let mut state = state.lock().unwrap();
                if let Ok(registry) = state.as_mut() {
                    let before = serde_json::to_value(registry.list()).unwrap();
                    apply_pending(registry, &pending);
                    for (id, worker) in &mut workers {
                        if let Ok(result) = worker.results.try_recv() {
                            worker.busy = false;
                            if let Ok(integration) = registry.get(id) {
                                integration.apply_poll(result);
                            }
                        }
                    }
                    let busy = workers
                        .iter()
                        .filter(|(_, w)| w.busy)
                        .map(|(id, _)| id.clone())
                        .collect();
                    for (id, poll) in registry.poll_jobs(&busy) {
                        let worker = workers
                            .entry(id.clone())
                            .or_insert_with(|| PollWorker::new(id));
                        worker.busy = worker.jobs.as_ref().unwrap().send(poll).is_ok();
                    }
                    *cache.lock().unwrap() = registry.list();
                    if before != serde_json::to_value(registry.list()).unwrap() {
                        publish(registry, &events);
                    }
                }
            }
        });
        *service.runtime.handle.lock().unwrap() = Some(handle);
        service
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn mock_volume(&self) -> Option<td_host::integrations::volume::fake::FakeVolume> {
        self.volume.clone()
    }
    pub fn summaries(&self) -> Vec<IntegrationSummary> {
        self.summaries.lock().unwrap().clone()
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn media_fake(&self) -> Option<td_host::integrations::media::fake::FakeMedia> {
        self.media.clone()
    }
    pub fn live(&self, wanted: &BTreeSet<StreamKey>) -> Option<HashMap<StreamKey, LiveSample>> {
        let state = self.registry.try_lock().ok()?;
        let Ok(registry) = state.as_ref() else {
            return Some(HashMap::new());
        };
        Some(
            registry
                .live()
                .filter(|(id, _)| {
                    !self.controls.contains_key(*id)
                        && wanted.iter().any(|key| key.integration == *id)
                })
                .flat_map(|(id, samples)| {
                    samples.into_iter().map(move |sample| {
                        let key = StreamKey {
                            integration: id.to_owned(),
                            account: sample.account.clone(),
                            stream: sample.stream.clone(),
                        };
                        (key, sample)
                    })
                })
                .filter(|(key, _)| wanted.contains(key))
                .collect(),
        )
    }
    /// Whether the Integration is turned on. Reads the cached summaries, never the registry.
    pub fn enabled(&self, id: &str) -> bool {
        self.summaries
            .lock()
            .unwrap()
            .iter()
            .any(|summary| summary.id == id && summary.enabled)
    }
    /// Whether a Device's input can set this Integration stream: its Integration lists it as `settable`. Reads the
    /// cached summaries, never the registry.
    pub fn settable(&self, key: &StreamKey) -> bool {
        self.summaries.lock().unwrap().iter().any(|summary| {
            summary.id == key.integration
                && summary
                    .streams
                    .iter()
                    .any(|stream| stream.id == key.stream && stream.settable)
        })
    }
    /// The newest samples of the wanted streams that come from Controls. Never waits for the registry.
    pub fn control_live(&self, wanted: &BTreeSet<StreamKey>) -> HashMap<StreamKey, LiveSample> {
        self.controls
            .iter()
            .filter(|(id, _)| wanted.iter().any(|k| &k.integration == *id))
            .flat_map(|(id, control)| {
                control.live().into_iter().map(|sample| {
                    (
                        StreamKey {
                            integration: id.clone(),
                            account: sample.account.clone(),
                            stream: sample.stream.clone(),
                        },
                        sample,
                    )
                })
            })
            .filter(|(key, _)| wanted.contains(key))
            .collect()
    }
    /// Records what the Companion delivered to a Device for an Integration. One that comes while the registry is busy
    /// is applied by the next caller that takes the registry.
    pub fn delivered(&self, integration: &str, delivery: Delivery) {
        self.pending
            .lock()
            .unwrap()
            .insert((integration.to_owned(), delivery.device.clone()), delivery);
        if let Ok(mut state) = self.registry.try_lock()
            && let Ok(registry) = state.as_mut()
        {
            let before = serde_json::to_value(registry.list()).unwrap();
            apply_pending(registry, &self.pending);
            *self.summaries.lock().unwrap() = registry.list();
            if before != serde_json::to_value(registry.list()).unwrap() {
                publish(registry, &self.events);
            }
        }
    }
    fn with_registry<T>(
        &self,
        f: impl FnOnce(&mut Registry) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut guard = self.registry.lock().unwrap();
        let registry = guard.as_mut().map_err(|_| Error::Credentials)?;
        let before = serde_json::to_value(registry.list()).unwrap();
        apply_pending(registry, &self.pending);
        let result = f(registry);
        *self.summaries.lock().unwrap() = registry.list();
        if before != serde_json::to_value(registry.list()).unwrap() {
            publish(registry, &self.events);
        }
        result
    }
}
fn configure(
    integration: &mut dyn td_host::integrations::Integration,
    mut config: Value,
) -> Result<IntegrationView, Error> {
    if let Some(fields) = config.as_object_mut()
        && !fields.contains_key("enabled")
    {
        fields.insert("enabled".into(), Value::Bool(integration.summary().enabled));
    }
    integration.configure(config)
}
fn apply_pending(registry: &mut Registry, pending: &Pending) {
    for ((integration, _), delivery) in pending.lock().unwrap().drain() {
        if let Ok(integration) = registry.get(&integration) {
            integration.delivered(delivery);
        }
    }
}
struct FakeFetch;
impl td_host::sources::Fetch for FakeFetch {
    fn get(&self, url: &str, bearer: &str) -> Result<String, String> {
        #[cfg(feature = "test-hooks")]
        if loopback_stub(url) {
            return td_host::sources::HttpFetch::new().get(url, bearer);
        }
        let _ = (url, bearer);
        Ok("[]".into())
    }
    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<String, String> {
        #[cfg(feature = "test-hooks")]
        if loopback_stub(url) {
            return td_host::sources::HttpFetch::new().post_json(url, bearer, body);
        }
        let _ = (url, bearer, body);
        Err("No fixture".into())
    }
}
#[cfg(feature = "test-hooks")]
fn loopback_stub(url: &str) -> bool {
    url.strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(port, _)| port.parse::<u16>().is_ok_and(|port| port > 0))
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct IntegrationStatusEvent(pub IntegrationSummary);
fn publish(registry: &Registry, events: &Events) {
    for summary in registry.list() {
        events.publish(&IntegrationStatusEvent(summary));
    }
}

pub mod session;

#[cfg(test)]
#[path = "integrations/tests.rs"]
mod tests;
