use crate::api::{
    ApiError, CommonError, Ctx, Endpoint, Event, Events, Feature, NoQuery, Registration, Req,
};
use crate::live::StreamKey;
use axum::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use td_host::integrations::secrets::{MemorySecrets, OsSecrets};
use td_host::integrations::{Control, Delivery, Environment, Error, LiveSample, Registry};
pub use td_host::integrations::{IntegrationSummary, IntegrationView};
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
impl Service {
    pub(crate) fn stop(&self) {
        self.runtime
            .stop
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.runtime.handle.lock().unwrap().as_ref() {
            handle.thread().unpark();
        }
    }

    pub fn command_approval(&self, approval: Arc<dyn td_host::integrations::CommandApproval>) {
        if let Ok(registry) = self.registry.lock().unwrap().as_mut()
            && let Ok(custom) = registry.get("custom")
        {
            custom.command_approval(approval);
        }
    }
    pub fn new(mock: bool, events: Events) -> Self {
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
        events: &Events,
        f: impl FnOnce(&mut Registry) -> Result<T, Error>,
    ) -> Result<T, IntegrationError> {
        let mut guard = self.registry.lock().unwrap();
        let registry = guard.as_mut().map_err(|_| IntegrationError::Credentials)?;
        let before = serde_json::to_value(registry.list()).unwrap();
        apply_pending(registry, &self.pending);
        let result = f(registry).map_err(IntegrationError::from);
        *self.summaries.lock().unwrap() = registry.list();
        if before != serde_json::to_value(registry.list()).unwrap() {
            publish(registry, events);
        }
        result
    }
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
impl Event for IntegrationStatusEvent {
    const NAME: &'static str = "integration.status";
}
fn publish(registry: &Registry, events: &Events) {
    for summary in registry.list() {
        events.publish(&IntegrationStatusEvent(summary));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("The Integration does not exist.")]
    NotFound,
    #[error("Invalid Integration config.")]
    ConfigInvalid,
    #[error("Invalid Integration config.")]
    ConfigField {
        field: &'static str,
        message: &'static str,
    },
    #[error("The credential store is unavailable.")]
    Credentials,
    #[error(transparent)]
    Common(#[from] CommonError),
}
impl From<Error> for IntegrationError {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound => Self::NotFound,
            Error::ConfigInvalid => Self::ConfigInvalid,
            Error::ConfigField { field, message } => Self::ConfigField { field, message },
            Error::Credentials => Self::Credentials,
            Error::Storage => Self::Common(CommonError::Internal),
        }
    }
}
impl ApiError for IntegrationError {
    const CODES: &'static [&'static str] = &[
        "integration.not_found",
        "integration.config_invalid",
        "integration.credentials",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ConfigInvalid | Self::ConfigField { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Credentials => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Common(e) => e.status(),
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::NotFound => Self::CODES[0],
            Self::ConfigInvalid | Self::ConfigField { .. } => Self::CODES[1],
            Self::Credentials => Self::CODES[2],
            Self::Common(e) => e.code(),
        }
    }
    fn details(&self) -> Option<Value> {
        match self {
            Self::ConfigInvalid => {
                Some(serde_json::json!({"issues":[{"message":"Check the Integration fields."}]}))
            }
            Self::ConfigField { field, message } => {
                Some(serde_json::json!({"issues":[{"path":field,"message":message}]}))
            }
            Self::Common(e) => e.details(),
            _ => None,
        }
    }
}
async fn work<T: Send + 'static>(
    ctx: Ctx,
    f: impl FnOnce(&mut Registry) -> Result<T, Error> + Send + 'static,
) -> Result<T, IntegrationError> {
    tokio::task::spawn_blocking(move || ctx.integrations.with_registry(&ctx.events, f))
        .await
        .map_err(|_| IntegrationError::Credentials)?
}
#[derive(Serialize, Deserialize, TS)]
pub struct IntegrationPath {
    pub integration: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct SecretPath {
    pub integration: String,
    pub field: String,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SecretInput {
    pub value: String,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct EnabledInput {
    pub enabled: bool,
}
#[derive(Serialize, Deserialize, TS)]
pub struct SecretHint {
    pub hint: String,
}

pub struct List;
impl Endpoint for List {
    const NAME: &'static str = "integrations.list";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/integrations";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = Vec<IntegrationSummary>;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, |r| Ok(r.list())).await
    }
}
pub struct Get;
impl Endpoint for Get {
    const NAME: &'static str = "integrations.get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/integrations/{integration}";
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = IntegrationView;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |r| r.get(&req.path.integration)?.view()).await
    }
}
pub struct ConfigPut;
impl Endpoint for ConfigPut {
    const NAME: &'static str = "integrations.config_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/integrations/{integration}/config";
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = Value;
    type Ok = IntegrationView;
    type Error = IntegrationError;
    /// A config object without `enabled` keeps the stored value, so a form that doesn't send it never undoes
    /// an `integrations.enabled_put` from another client.
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |r| {
            let integration = r.get(&req.path.integration)?;
            let mut config = req.body;
            if let Some(fields) = config.as_object_mut()
                && !fields.contains_key("enabled")
            {
                fields.insert("enabled".into(), Value::Bool(integration.summary().enabled));
            }
            integration.configure(config)
        })
        .await
    }
}
pub struct EnabledPut;
impl Endpoint for EnabledPut {
    const NAME: &'static str = "integrations.enabled_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/integrations/{integration}/enabled";
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = EnabledInput;
    type Ok = IntegrationView;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |r| {
            r.get(&req.path.integration)?.set_enabled(req.body.enabled)
        })
        .await
    }
}
pub struct SecretPut;
impl Endpoint for SecretPut {
    const NAME: &'static str = "integrations.secret_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/integrations/{integration}/secrets/{field}";
    type Path = SecretPath;
    type Query = NoQuery;
    type Body = SecretInput;
    type Ok = SecretHint;
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |r| {
            r.get(&req.path.integration)?
                .secret_put(&req.path.field, &req.body.value)?;
            Ok(SecretHint { hint: "Set".into() })
        })
        .await
    }
}
pub struct SecretDelete;
impl Endpoint for SecretDelete {
    const NAME: &'static str = "integrations.secret_delete";
    const METHOD: Method = Method::DELETE;
    const PATH: &'static str = "/integrations/{integration}/secrets/{field}";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = SecretPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |r| {
            r.get(&req.path.integration)?.secret_delete(&req.path.field)
        })
        .await
    }
}
pub struct Refresh;
impl Endpoint for Refresh {
    const NAME: &'static str = "integrations.refresh";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/integrations/{integration}/refresh";
    const STATUS: StatusCode = StatusCode::NO_CONTENT;
    type Path = IntegrationPath;
    type Query = NoQuery;
    type Body = ();
    type Ok = ();
    type Error = IntegrationError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        work(ctx, move |r| {
            r.get(&req.path.integration)?.request_refresh();
            Ok(())
        })
        .await
    }
}
fn feature() -> Feature {
    Feature::new("integrations")
        .endpoint::<List>()
        .endpoint::<Get>()
        .endpoint::<ConfigPut>()
        .endpoint::<EnabledPut>()
        .endpoint::<SecretPut>()
        .endpoint::<SecretDelete>()
        .endpoint::<Refresh>()
        .event::<IntegrationStatusEvent>()
        .snapshot("integrations", |ctx| ctx.integrations.summaries())
}
inventory::submit! { Registration(feature) }

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(service: &Service, events: &Events) -> Vec<(String, u64)> {
        let list = service.with_registry(events, |r| Ok(r.list())).unwrap();
        let this_pc = list.into_iter().find(|i| i.id == "this_pc").unwrap();
        this_pc
            .status
            .delivery
            .into_iter()
            .map(|d| (d.device, d.sent))
            .collect()
    }

    #[test]
    fn a_delivery_reported_while_the_integrations_are_busy_still_shows() {
        let events = Events::default();
        let service = Service::new(true, events.clone());
        let busy = service.registry.lock().unwrap();
        for sent in 3..=4 {
            service.delivered(
                "this_pc",
                Delivery {
                    device: "mock0001".into(),
                    sent,
                    error: None,
                },
            );
        }
        drop(busy);

        assert_eq!(sent(&service, &events), [("mock0001".to_owned(), 4)]);
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use crate::devices::{Config, DeviceManager, LinkState};
    use crate::link::Links;
    use crate::live::Live;
    use crate::live::fake::FakeControl;
    use std::time::{Duration, Instant};
    use td_core::live::LiveValue;

    struct Running {
        service: Service,
        fake: Arc<FakeControl>,
        deck: td_link::MockDeck,
        devices: DeviceManager,
        live: Option<Live>,
    }

    impl Drop for Running {
        fn drop(&mut self) {
            self.live.take();
            self.devices.stop();
        }
    }

    fn wait(what: &str, within: Duration, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + within;
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn running(integration: &str, layout: serde_json::Value) -> Running {
        let events = Events::default();
        let devices = DeviceManager::new(events.clone());
        let config = Config::mock(None);
        let ports = config.mock.as_ref().unwrap().0.clone();
        ports.plug_media_device("mock0001");
        let deck = ports.deck("mock0001").unwrap();
        devices.start(config);
        wait("the Device connects", Duration::from_secs(5), || {
            devices
                .summaries()
                .iter()
                .any(|d| matches!(d.link, LinkState::Connected { .. }))
        });
        let links = Links::new(devices.clone());
        let layout = td_core::Layout::parse(&serde_json::to_vec(&layout).unwrap()).unwrap();
        links
            .with("mock0001", |s| {
                td_link::xfer::put_layout(s, layout.orientation, &layout)
            })
            .unwrap()
            .unwrap();
        let mut service = Service::new(true, events);
        let fake = Arc::new(FakeControl::default());
        service.controls = Arc::new(BTreeMap::from([(
            integration.to_owned(),
            fake.clone() as Arc<dyn Control>,
        )]));
        let data = tempfile::tempdir().unwrap();
        let authority = crate::layouts::LayoutAuthority::new(
            crate::store::LayoutStore::new(data.path().to_owned()),
            Arc::new(|_| false),
            devices.clone(),
        );
        let live = Live::start(devices.clone(), links, service.clone(), authority);
        Running {
            service,
            fake,
            deck,
            devices,
            live: Some(live),
        }
    }

    fn graph() -> serde_json::Value {
        let mut layout: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../../../../schema/examples/graph.json"))
                .unwrap();
        layout["pages"][0]["tiles"][0]["action"] =
            serde_json::json!({"type":"widget","source":"test","account":"","field":"level"});
        layout
    }

    fn shown(deck: &td_link::MockDeck) -> Option<serde_json::Value> {
        deck.live_sets().last().map(|set| set["value"].clone())
    }

    #[test]
    fn only_a_stream_its_integration_lists_as_settable_takes_input() {
        let service = Service::new(true, Events::default());
        let key = |integration: &str, stream: &str| StreamKey {
            integration: integration.into(),
            account: String::new(),
            stream: stream.into(),
        };
        let busy = service.registry.lock().unwrap();
        let mut summaries = service.summaries.lock().unwrap();
        let mut volume = summaries[0].clone();
        volume.id = "volume".into();
        volume.streams = vec![
            td_host::integrations::DataStream {
                id: "level".into(),
                kind: "number".into(),
                unit: None,
                settable: true,
            },
            td_host::integrations::DataStream {
                id: "muted".into(),
                kind: "number".into(),
                unit: None,
                settable: false,
            },
        ];
        summaries.push(volume);
        drop(summaries);
        assert!(service.settable(&key("volume", "level")));
        assert!(!service.settable(&key("volume", "muted")));
        assert!(!service.settable(&key("volume", "other")));
        assert!(!service.settable(&key("media", "level")));
        assert!(!service.settable(&key("this_pc", "cpu")));
        drop(busy);
    }

    #[test]
    fn control_samples_are_read_while_the_registry_is_busy() {
        let mut service = Service::new(true, Events::default());
        let fake = Arc::new(FakeControl::default());
        service.controls = Arc::new(BTreeMap::from([(
            "test".to_owned(),
            fake.clone() as Arc<dyn Control>,
        )]));
        let key = StreamKey {
            integration: "test".into(),
            account: String::new(),
            stream: "level".into(),
        };
        let busy = service.registry.lock().unwrap();
        fake.number(0.4);
        assert_eq!(service.live(&BTreeSet::from([key.clone()])), None);
        assert_eq!(
            service.control_live(&BTreeSet::from([key.clone()]))[&key].value,
            LiveValue::Number(0.4)
        );
        drop(busy);
    }

    #[test]
    fn a_woken_sample_reaches_the_device_before_the_poll_releases_the_registry() {
        let run = running("test", graph());
        run.fake.number(0.1);
        wait("the first value arrives", Duration::from_secs(5), || {
            shown(&run.deck) == Some(serde_json::json!(0.1))
        });
        let registry = run.service.registry.lock().unwrap();
        for value in [0.7, 0.3, 0.9] {
            run.fake.number(value);
            wait(
                "the woken value arrives while the registry is held",
                Duration::from_secs(5),
                || shown(&run.deck) == Some(serde_json::json!(value)),
            );
        }
        drop(registry);
    }

    #[test]
    fn a_track_reaches_the_device_with_its_cover_made_off_the_link() {
        let run = running(
            "media",
            serde_json::json!({
                "schema": 2, "name": "Media", "orientation": "landscape",
                "pages": [{"kind": "grid", "id": "main", "title": "Media", "rows": 1, "cols": 1, "tiles": [{
                    "id": "player", "icon": null, "label": "Player", "row": 0, "col": 0, "rowSpan": 1,
                    "colSpan": 1, "render": "now-playing",
                    "streams": {"title": "text", "artist": "text", "cover": "image", "state": "number"},
                    "action": {"type": "widget", "source": "media", "account": "", "field": "title"}
                }]}]
            }),
        );
        run.fake.add_picture(
            "sha256:art",
            crate::photos::encode::tests::noise_png(640, 480, 5),
        );
        run.fake.publish(&[
            ("title", LiveValue::Text("Ło\u{301}dź".into())),
            ("artist", LiveValue::Text("Artist".into())),
            ("state", LiveValue::Number(2.0)),
            ("cover", LiveValue::Image(Some("sha256:art".into()))),
        ]);
        let value = |stream: &str| run.deck.live("main", "player", stream).map(|l| l.value);
        wait("the cover arrives", Duration::from_secs(5), || {
            value("cover").is_some_and(|v| v.is_string())
        });
        assert_eq!(value("title"), Some(serde_json::json!("Lódz")));
        let cover = value("cover").unwrap();
        let id = cover.as_str().unwrap().strip_prefix("sha256:").unwrap();
        let picture = image::load_from_memory(&run.deck.assets()[id]).unwrap();
        assert_eq!((picture.width(), picture.height()), (256, 256));
    }
}

#[cfg(test)]
mod poll_isolation_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    #[derive(Clone, Copy)]
    struct Upstream {
        delay: Duration,
        panic_first: bool,
    }
    const SLOW: Upstream = Upstream {
        delay: Duration::from_secs(3),
        panic_first: false,
    };
    const SLOWER: Upstream = Upstream {
        delay: Duration::from_secs(5),
        panic_first: false,
    };
    #[derive(Default)]
    struct Calls {
        started: AtomicUsize,
        finished: AtomicUsize,
    }
    impl Calls {
        fn started(&self) -> usize {
            self.started.load(Ordering::SeqCst)
        }
        fn finished(&self) -> usize {
            self.finished.load(Ordering::SeqCst)
        }
    }
    struct Fetch(Arc<Calls>, Upstream);
    impl td_host::sources::Fetch for Fetch {
        fn post_json(&self, _: &str, _: &str, _: &Value) -> Result<String, String> {
            Ok("[]".into())
        }
        fn get(&self, _: &str, _: &str) -> Result<String, String> {
            Ok("[]".into())
        }
        fn request(
            &self,
            request: &td_host::sources::network::GetRequest<'_>,
        ) -> Result<String, td_host::sources::network::FetchError> {
            let call = self.0.started.fetch_add(1, Ordering::SeqCst);
            assert!(
                !(self.1.panic_first && call == 0),
                "upstream fixture fails its first call"
            );
            std::thread::sleep(self.1.delay);
            self.0.finished.fetch_add(1, Ordering::SeqCst);
            Ok(if request.url.ends_with("/old") {
                "1"
            } else {
                "2"
            }
            .into())
        }
    }
    struct Counter;
    impl td_host::integrations::this_pc::CounterSource for Counter {
        fn collect(&mut self) -> td_host::integrations::this_pc::CounterSample {
            td_host::integrations::this_pc::CounterSample {
                values: BTreeMap::from([("cpu_pct".into(), 42.0)]),
                ..Default::default()
            }
        }
    }
    fn config(enabled: bool, path: &str) -> Value {
        serde_json::json!({"enabled": enabled, "sources": [{"id":"value", "name":"Value", "interval_s":3600, "kind":"number", "input":{"type":"http", "url":format!("https://example.invalid/{path}"), "value_path":"$"}}]})
    }
    fn start() -> (Service, Arc<Calls>) {
        start_with(SLOW)
    }
    fn start_with(upstream: Upstream) -> (Service, Arc<Calls>) {
        let calls = Arc::new(Calls::default());
        let fetch = calls.clone();
        let mut registry = Registry::new(Environment {
            media_provider: Some(Arc::new(
                td_host::integrations::media::fake::FakeMedia::default(),
            )),
            volume: Some(td_host::integrations::volume::fake::FakeVolume::default().factory()),
            secrets: Arc::new(MemorySecrets::default()),
            config_file: None,
            fetch: Arc::new(move || Box::new(Fetch(fetch.clone(), upstream))),
        })
        .unwrap();
        registry
            .get("custom")
            .unwrap()
            .configure(config(true, "old"))
            .unwrap();
        *registry.get("this_pc").unwrap() = Box::new(td_host::integrations::this_pc::ThisPc::new(
            Box::new(Counter),
        ));
        let service = Service::new(true, Events::default());
        *service.registry.lock().unwrap() = Ok(registry);
        assert!(
            wait_until(Duration::from_secs(5), || calls.started() > 0),
            "the first poll never started"
        );
        (service, calls)
    }
    fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if done() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn this_pc_sample(service: &Service) -> Option<u64> {
        let key = StreamKey {
            integration: "this_pc".into(),
            account: String::new(),
            stream: "cpu_pct".into(),
        };
        let values = service.live(&BTreeSet::from([key.clone()]))?;
        Some(values.get(&key)?.sample)
    }
    fn this_pc_advances(service: &Service, times: usize) -> bool {
        let mut seen = None;
        let mut advances = 0;
        wait_until(Duration::from_secs(5), || {
            if let Some(sample) = this_pc_sample(service) {
                if seen.is_some_and(|seen| sample > seen) {
                    advances += 1;
                }
                if seen.is_none_or(|seen| sample > seen) {
                    seen = Some(sample);
                }
            }
            advances >= times
        })
    }
    fn custom_values(service: &Service) -> Vec<td_core::live::LiveValue> {
        service
            .with_registry(&service.events, |r| Ok(r.get("custom")?.live()))
            .unwrap()
            .into_iter()
            .map(|sample| sample.value)
            .collect()
    }
    fn refresh_custom(service: &Service) {
        service
            .with_registry(&service.events, |r| {
                r.get("custom")?.request_refresh();
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn slow_poll_does_not_block_registry_calls() {
        let (service, _) = start();
        type Operation = fn(&mut Registry) -> Result<(), Error>;
        let operations: [Operation; 6] = [
            |r| {
                r.list();
                Ok(())
            },
            |r| {
                r.get("custom")?.view()?;
                Ok(())
            },
            |r| {
                r.get("custom")?.request_refresh();
                Ok(())
            },
            |r| {
                r.get("cliproxyapi")?
                    .secret_put("management_key", "fixture-private-value")
            },
            |r| r.get("cliproxyapi")?.secret_delete("management_key"),
            |r| {
                r.get("custom")?.configure(config(true, "new"))?;
                Ok(())
            },
        ];
        for operation in operations {
            let at = Instant::now();
            service.with_registry(&service.events, operation).unwrap();
            assert!(
                at.elapsed() < Duration::from_millis(200),
                "registry call took {:?}",
                at.elapsed()
            );
        }
    }
    #[test]
    fn dropping_service_joins_in_flight_workers() {
        let (service, _) = start();
        let stop = service.runtime.stop.clone();
        let at = Instant::now();
        drop(service);
        assert!(stop.load(std::sync::atomic::Ordering::Acquire));
        assert!(at.elapsed() >= Duration::from_secs(2));
        assert!(at.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn slow_poll_does_not_delay_other_live_values() {
        let (service, calls) = start_with(SLOWER);
        assert!(
            this_pc_advances(&service, 2),
            "This PC samples stopped advancing during the slow poll"
        );
        assert_eq!(
            calls.finished(),
            0,
            "the slow poll ended before This PC advanced twice"
        );
        assert_eq!(calls.started(), 1);
    }
    fn change_during_poll(enabled: bool) {
        let (service, calls) = start();
        let at = Instant::now();
        let view = service
            .with_registry(&service.events, |r| {
                r.get("custom")?.configure(config(enabled, "new"))
            })
            .unwrap();
        assert_eq!(view.summary.enabled, enabled);
        assert!(
            at.elapsed() < Duration::from_millis(200),
            "config change took {:?}",
            at.elapsed()
        );
        assert!(wait_until(Duration::from_secs(5), || calls.finished() > 0));
        assert!(
            this_pc_advances(&service, 2),
            "the loop stopped ticking after the old poll ended"
        );
        let summary = service
            .with_registry(&service.events, |r| Ok(r.get("custom")?.summary()))
            .unwrap();
        assert_eq!(summary.enabled, enabled);
        assert_eq!(
            summary.revision, view.summary.revision,
            "an old poll result rewound the summary"
        );
        assert!(
            !custom_values(&service).contains(&td_core::live::LiveValue::Number(1.0)),
            "an old poll result became a live value"
        );
    }
    #[test]
    fn disable_drops_in_flight_result() {
        change_during_poll(false);
    }
    #[test]
    fn config_edit_drops_in_flight_result() {
        change_during_poll(true);
    }
    #[test]
    fn only_one_poll_is_in_flight() {
        let (service, calls) = start_with(SLOWER);
        let until = Instant::now() + Duration::from_millis(3500);
        while Instant::now() < until {
            let at = Instant::now();
            refresh_custom(&service);
            assert!(
                at.elapsed() < Duration::from_millis(200),
                "refresh took {:?} while one poll was in flight",
                at.elapsed()
            );
            assert_eq!(calls.started(), 1);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    #[test]
    fn refresh_during_poll_polls_again_when_it_ends() {
        let (service, calls) = start();
        refresh_custom(&service);
        assert_eq!(calls.started(), 1);
        assert!(
            wait_until(Duration::from_secs(8), || calls.started() > 1),
            "a refresh sent during a poll was lost"
        );
        assert_eq!(calls.finished(), 1, "the second poll overlapped the first");
    }
    #[test]
    fn a_panicking_poll_is_retried() {
        let (service, _) = start_with(Upstream {
            delay: Duration::ZERO,
            panic_first: true,
        });
        assert!(
            wait_until(Duration::from_secs(8), || !custom_values(&service)
                .is_empty()),
            "polling stopped after one poll panicked"
        );
    }
}
