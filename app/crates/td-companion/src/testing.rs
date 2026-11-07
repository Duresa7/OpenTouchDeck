use crate::{
    actions::{FileGrants, GrantStore},
    api::{Ctx, Endpoint},
    devices::{
        BoardKind, Config, DeviceId, DeviceSummary, InputSuppressed, LinkState, MockPorts, NewBoard,
    },
};
use futures_util::StreamExt;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::client::IntoClientRequest};
/// How long a `plug` or `unplug` helper waits for the Companion to notice.
const SETTLE: Duration = Duration::from_secs(5);
enum Ports {
    Mock,
    Real { device_console: bool },
}
pub struct TestCompanion {
    ctx: Ctx,
    ports: MockPorts,
    input: td_host::FakeSink,
    _data: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
    pub token: String,
    pub pairing_token: String,
    pub url: String,
    client: reqwest::Client,
}
impl TestCompanion {
    pub fn control_fakes(
        &self,
    ) -> (
        td_host::integrations::volume::fake::FakeVolume,
        td_host::integrations::media::fake::FakeMedia,
    ) {
        (
            self.ctx.integrations.mock_volume().unwrap(),
            self.ctx.integrations.media_fake().unwrap(),
        )
    }
    pub async fn start() -> Self {
        Self::start_with(crate::Limits::default()).await
    }
    /// Starts with other connection and time limits, so limit tests run in milliseconds.
    pub async fn start_with(limits: crate::Limits) -> Self {
        Self::start_router(
            limits,
            false,
            None,
            None,
            Box::new(NoCatalogNetwork),
            None,
            Ports::Mock,
        )
        .await
    }
    pub async fn start_with_catalog(
        limits: crate::Limits,
        fetcher: Box<dyn td_host::catalog::Fetcher>,
    ) -> Self {
        Self::start_router(limits, false, None, None, fetcher, None, Ports::Mock).await
    }
    /// Starts offering `curated` as the Icon pack list, downloaded through `fetcher`.
    pub async fn start_with_packs(
        curated: Vec<td_host::packs::Curated>,
        fetcher: Box<dyn td_host::catalog::Fetcher>,
    ) -> Self {
        Self::start_router(
            crate::Limits::default(),
            false,
            None,
            None,
            Box::new(NoCatalogNetwork),
            Some((curated, fetcher)),
            Ports::Mock,
        )
        .await
    }
    pub async fn start_release_router() -> Self {
        Self::start_router(
            crate::Limits::default(),
            true,
            None,
            None,
            Box::new(NoCatalogNetwork),
            None,
            Ports::Mock,
        )
        .await
    }
    /// Starts on mock ports that the Companion treats as real ones, as a Companion without
    /// `--mock-devices` does, with or without `--test-real-device-console`.
    pub async fn start_on_real_ports(device_console: bool) -> Self {
        Self::start_router(
            crate::Limits::default(),
            false,
            None,
            None,
            Box::new(NoCatalogNetwork),
            None,
            Ports::Real { device_console },
        )
        .await
    }
    /// Starts with grants kept in `grants` instead of a file.
    pub async fn start_with_grants(grants: Arc<dyn GrantStore>) -> Self {
        Self::start_router(
            crate::Limits::default(),
            false,
            Some(grants),
            None,
            Box::new(NoCatalogNetwork),
            None,
            Ports::Mock,
        )
        .await
    }
    async fn start_router(
        limits: crate::Limits,
        release: bool,
        grants: Option<Arc<dyn GrantStore>>,
        desktop: Option<Arc<crate::tray::Desktop>>,
        fetcher: Box<dyn td_host::catalog::Fetcher>,
        packs: Option<(
            Vec<td_host::packs::Curated>,
            Box<dyn td_host::catalog::Fetcher>,
        )>,
        mode: Ports,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let data = tempfile::tempdir().unwrap();
        let path = data.path().to_owned();
        let mut ctx = tokio::task::spawn_blocking(move || {
            let grants = grants
                .unwrap_or_else(|| Arc::new(FileGrants::new(path.join("allowed-actions.json"))));
            Ctx::new(port, true, None, limits, &path, grants)
        })
        .await
        .unwrap();
        ctx.catalog = std::sync::Arc::new(td_host::catalog::Catalog::new(
            data.path().join("cache/catalog"),
            fetcher,
        ));
        let (curated, pack_fetcher): (_, Box<dyn td_host::catalog::Fetcher>) = match packs {
            Some(packs) => packs,
            None => (
                td_host::packs::curated().to_vec(),
                Box::new(NoCatalogNetwork),
            ),
        };
        ctx.packs = std::sync::Arc::new(td_host::packs::Packs::with_list(
            data.path().join("packs"),
            pack_fetcher,
            curated,
        ));
        if let Some(desktop) = desktop {
            ctx.desktop = desktop;
        }
        let mut config = Config::mock(Some(data.path().join("devices.json")));
        let (ports, input) = config.mock.clone().unwrap();
        if let Ports::Real { device_console } = mode {
            config.mock = None;
            ctx.real_device_console = device_console;
        }
        ctx.devices.start(config);
        let router = if release {
            crate::api::release_router(ctx.clone())
        } else {
            crate::api::router(ctx.clone())
        };
        let stop = ctx.shutdown.subscribe();
        let task = tokio::spawn(crate::serve::serve(listener, router, limits, stop));
        let url = format!("http://127.0.0.1:{port}");
        // Below the 5 s header-read limit, so a pooled connection is never reused as the
        // Companion closes it.
        let client = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(4))
            .build()
            .unwrap();
        let pairing_token = ctx.pairing_token();
        let session: serde_json::Value = client
            .post(format!("{url}/api/v1/auth/sessions"))
            .bearer_auth(&pairing_token)
            .json(&serde_json::json!({"kind":"window"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            ctx,
            ports,
            input,
            _data: data,
            task,
            token: session["token"].as_str().unwrap().into(),
            pairing_token,
            url,
            client,
        }
    }
    pub async fn restart(&mut self) {
        let desktop = self.ctx.desktop.clone();
        assert!(self.stop(Duration::from_secs(5)).await);
        *self = Self::start_router(
            crate::Limits::default(),
            false,
            None,
            Some(desktop),
            Box::new(NoCatalogNetwork),
            None,
            Ports::Mock,
        )
        .await;
    }
    pub fn pairing_delivery(&self) -> (Option<String>, Option<String>) {
        let (opener, notifier) = self.ctx.desktop.captures.as_ref().unwrap();
        (opener.0.lock().take(), notifier.0.lock().take())
    }
    /// Plugs in a mock Device with v2 Firmware under this USB serial and waits until it's
    /// connected.
    pub async fn plug_media_device(&self, id: &str) -> DeviceId {
        self.ports.plug_media_device(id);
        let id = DeviceId::parse(id).unwrap();
        self.until(|td| {
            td.device(&id)
                .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
        })
        .await;
        id
    }
    pub async fn plug_device(&self, id: &str) -> DeviceId {
        self.plug(id, BoardKind::Device).await
    }
    /// Plugs in a mock Device with v1 Firmware and waits until it's connected.
    pub async fn plug_device_v1(&self, id: &str) -> DeviceId {
        self.plug(id, BoardKind::DeviceV1).await
    }
    async fn plug(&self, id: &str, kind: BoardKind) -> DeviceId {
        self.ports.plug(id, kind);
        let id = DeviceId::parse(id).expect("a valid Device id");
        self.until(|td| {
            td.device(&id)
                .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
        })
        .await;
        id
    }
    /// Plugs in a board that isn't a Device and waits until it's a New board.
    pub async fn plug_board(&self, id: &str, kind: BoardKind) -> NewBoard {
        self.ports.plug(id, kind);
        let mut found = None;
        self.until(|td| {
            found = td.ctx.devices.new_boards().into_iter().find(|b| b.id == id);
            found.is_some()
        })
        .await;
        found.unwrap()
    }
    /// Unplugs a mock board and waits until the Companion has noticed.
    pub async fn unplug(&self, id: &str) {
        self.ports.unplug(id);
        self.until(|td| {
            !td.ctx
                .devices
                .ports()
                .iter()
                .any(|p| p.usb_serial.as_deref() == Some(id))
        })
        .await;
    }
    /// Waits up to 5 s for `done`, polling every 10 ms.
    pub async fn until(&self, mut done: impl FnMut(&Self) -> bool) {
        let started = Instant::now();
        while !done(self) {
            assert!(
                started.elapsed() < SETTLE,
                "the Companion didn't settle in time"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    pub fn device(&self, id: &DeviceId) -> Option<DeviceSummary> {
        self.ctx.devices.get(id)
    }
    /// How many Devices the Companion knows.
    pub fn device_count(&self) -> usize {
        self.ctx.devices.summaries().len()
    }
    pub fn firmware_prompter(
        &self,
        prompter: std::sync::Arc<dyn crate::features::firmware::Prompter>,
    ) {
        self.ctx.firmware.set_prompter(prompter);
    }
    /// Reports a live sample as if the Integration had read it, ahead of what the Integration reports itself.
    pub fn live_sample(&self, integration: &str, sample: td_host::integrations::LiveSample) {
        let key = crate::live::StreamKey {
            integration: integration.into(),
            account: sample.account.clone(),
            stream: sample.stream.clone(),
        };
        self.ctx.live.inject(key, sample);
    }
    /// The points in Device writes a test can hold.
    pub fn checkpoints(&self) -> &crate::checkpoints::Checkpoints {
        &self.ctx.checkpoints
    }
    /// Waits until `count` more live rounds have finished.
    pub async fn let_live_rounds_run(&self, count: u64) {
        let target = self.ctx.live.rounds() + count;
        self.until(|td| td.ctx.live.rounds() >= target).await;
    }
    /// Writes `layout` to the Device over the Companion's link without telling the live router, as a writer the
    /// Companion doesn't know about would.
    pub async fn write_layout_unseen(&self, id: &DeviceId, layout: &td_core::Layout) {
        let links = self.ctx.links.clone();
        let (id, layout) = (id.clone(), layout.clone());
        tokio::task::spawn_blocking(move || {
            links
                .with(id.as_str(), |session| {
                    td_link::xfer::put_layout(session, layout.orientation, &layout)
                })
                .unwrap()
                .unwrap();
        })
        .await
        .unwrap();
    }
    /// The mock behind a USB serial, to drive it or read what it received.
    pub fn deck(&self, id: &str) -> td_link::MockDeck {
        self.ports.deck(id).expect("a plugged mock")
    }
    /// A fake net-v2 Device on loopback in front of the mock with this USB serial, made if
    /// needed but not plugged in. Mock discovery finds it while `announce` is on. Mock
    /// Devices report the id `020000000001`, so use that serial for USB and Wi-Fi to merge.
    pub fn wifi_device(&self, serial: &str, announce: bool) -> td_link::net::fake::FakeNetDevice {
        let deck = self.ports.board(serial, BoardKind::Device);
        let fake = td_link::net::fake::FakeNetDevice::start(deck);
        if announce {
            self.wifi_network()
                .announce(td_link::net::mdns::Discovered {
                    device_id: fake.device_id(),
                    name: format!("Mock Device {serial}"),
                    address: fake.addr().into(),
                    paired: Some(0),
                });
        }
        fake
    }
    /// The in-memory keychain of Pairing keys.
    pub fn wifi_keys(&self) -> crate::devices::MemoryKeys {
        self.ctx.devices.wifi_mock().expect("mock Wi-Fi").0.clone()
    }
    /// What mock discovery finds.
    pub fn wifi_network(&self) -> crate::devices::MockNetwork {
        self.ctx.devices.wifi_mock().expect("mock Wi-Fi").1.clone()
    }
    /// Pairs with `fake` through the API, as a person reading the code off the Device would,
    /// and waits until the Device is connected over Wi-Fi.
    pub async fn pair_wifi(&self, fake: &td_link::net::fake::FakeNetDevice) -> DeviceId {
        use crate::features::wifi::{
            AttemptPath, PairCode, PairCodeEndpoint, PairStart, PairStartEndpoint,
        };
        fake.open_window()
            .expect("the Device opens a pairing window");
        let started = self
            .call::<PairStartEndpoint>(
                (),
                crate::api::NoQuery,
                PairStart {
                    address: fake.addr().to_string(),
                },
            )
            .await
            .unwrap();
        let code = fake.code().expect("the Device shows a code");
        self.call::<PairCodeEndpoint>(
            AttemptPath {
                attempt: started.attempt,
            },
            crate::api::NoQuery,
            PairCode { code },
        )
        .await
        .unwrap();
        let id = started.device;
        self.until(|td| {
            td.device(&id).is_some_and(|d| {
                d.carrier == Some(crate::devices::Carrier::Wifi)
                    && matches!(d.link, LinkState::Connected { .. })
            })
        })
        .await;
        id
    }
    /// What the fake input sink typed, as `down <key>` and `up <key>`.
    pub fn input_log(&self) -> Vec<String> {
        crate::features::test_hooks::input_log(&self.input)
    }
    /// The text of every approval dialog the fake prompter showed, oldest first.
    pub fn prompted(&self) -> Vec<String> {
        self.ctx.actions.prompted()
    }
    /// Holds input suppressed, as an approval or Firmware dialog does.
    pub fn dialog(&self) -> InputSuppressed {
        self.ctx.devices.suppress_input()
    }
    /// How many requests reached a handler. A rejected request leaves it unchanged.
    pub fn handler_hits(&self) -> usize {
        self.ctx
            .handler_hits
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Asks the Companion to stop and reports whether its server finished within `within`.
    pub async fn stop(&mut self, within: std::time::Duration) -> bool {
        let _ = self.ctx.shutdown.send(true);
        tokio::time::timeout(within, &mut self.task).await.is_ok()
    }
    /// Runs one 60 s maintenance step now.
    pub async fn maintain(&self) {
        assert!(crate::process::maintain(&self.ctx).await);
    }
    /// Whether anything asked the Companion to stop.
    pub fn shutdown_requested(&self) -> bool {
        *self.ctx.shutdown.borrow()
    }
    pub fn browser_token(&self) -> String {
        self.ctx.browser_token()
    }
    pub fn current_pairing_token(&self) -> String {
        self.ctx.pairing_token()
    }
    pub fn clock(&self) -> TestClock {
        TestClock(self.ctx.clone())
    }
    pub fn flood_state(&self, frames: usize, bytes: usize) {
        for _ in 0..frames {
            self.ctx.events.publish_frame(
                serde_json::json!({"type": "device.updated", "data": "x".repeat(bytes)}),
                crate::api::Topic::State,
                None,
            );
        }
    }
    pub fn raw(&self) -> RawClient {
        RawClient {
            client: self.client.clone(),
            url: self.url.clone(),
        }
    }
    /// Makes Jobs that start from now on wait, running, until `release_jobs`.
    pub fn hold_jobs(&self) {
        self.ctx.jobs.hold();
    }
    pub fn release_jobs(&self) {
        self.ctx.jobs.release();
    }
    /// Reads the Job until it finishes, as `companion.jobs.wait` does.
    pub async fn wait(&self, job: &crate::jobs::Job) -> crate::jobs::Job {
        use crate::features::jobs::{Get, JobPath};
        let path = || JobPath {
            job: job.id.clone(),
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let job = self
                    .call::<Get>(path(), crate::api::NoQuery, ())
                    .await
                    .unwrap();
                if job.state.finished() {
                    return job;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the Job finishes within 10 s")
    }
    /// A typed request with the session token, for tests that add headers or read errors.
    pub fn request<E: Endpoint>(
        &self,
        path: E::Path,
        query: E::Query,
        body: &E::Body,
    ) -> reqwest::RequestBuilder {
        let mut route = E::PATH.to_owned();
        if let Some(fields) = serde_json::to_value(path).unwrap().as_object() {
            for (k, v) in fields {
                route = route.replace(&format!("{{{k}}}"), v.as_str().unwrap());
            }
        }
        let mut req = self
            .client
            .request(E::METHOD, format!("{}/api/v1{route}", self.url))
            .bearer_auth(&self.token);
        if !serde_json::to_value(&query).unwrap().is_null() {
            req = req.query(&query);
        }
        if !serde_json::to_value(body).unwrap().is_null() {
            req = req.json(body);
        }
        req
    }
    pub async fn call<E: Endpoint>(
        &self,
        path: E::Path,
        query: E::Query,
        body: E::Body,
    ) -> Result<E::Ok, reqwest::Error> {
        let response = self
            .request::<E>(path, query, &body)
            .send()
            .await?
            .error_for_status()?;
        if response.status() == 204 {
            return Ok(serde_json::from_value(serde_json::Value::Null).unwrap());
        }
        response.json().await
    }
    pub async fn events(&self) -> WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>> {
        let mut request = format!("{}/api/v1/events", self.url.replace("http:", "ws:"))
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("touchdeck.v1, touchdeck.bearer.{}", self.token)
                .parse()
                .unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        for expected in ["hello", "sync"] {
            let frame = socket.next().await.unwrap().unwrap();
            let value: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(value["type"], expected);
        }
        socket
    }
}
impl Drop for TestCompanion {
    fn drop(&mut self) {
        self.ctx.devices.stop();
        self.task.abort();
    }
}
pub struct RawClient {
    client: reqwest::Client,
    url: String,
}
impl RawClient {
    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client.get(format!("{}{path}", self.url))
    }
    pub fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.client.post(format!("{}{path}", self.url))
    }
    pub fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client.request(method, format!("{}{path}", self.url))
    }
}

pub struct TestClock(Ctx);
impl TestClock {
    /// Moves the Companion's monotonic clock, which session expiry reads.
    pub fn advance(&self, duration: std::time::Duration) {
        self.0.clock.advance(duration);
    }
}

struct NoCatalogNetwork;
impl td_host::catalog::Fetcher for NoCatalogNetwork {
    fn fetch(&self, _: &str) -> Result<Vec<u8>, td_host::catalog::Error> {
        Err(td_host::catalog::Error::Fetch(
            "Network disabled in tests".into(),
        ))
    }
}
