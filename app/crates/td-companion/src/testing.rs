mod in_process;
pub use in_process::InProcessCompanion;

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
/// How [`TestCompanion::start_router`] sets up a Companion. The default is what
/// [`TestCompanion::start`] runs.
struct StartOptions {
    limits: crate::Limits,
    asset_store_bytes: u64,
    /// Serve the release router instead of the one with test routes.
    release: bool,
    /// Grants kept here instead of in a file.
    grants: Option<Arc<dyn GrantStore>>,
    /// The desktop to keep, as a restart does.
    desktop: Option<Arc<crate::tray::Desktop>>,
    /// Where catalog images are fetched from.
    catalog: Box<dyn td_host::catalog::Fetcher>,
    /// The Icon pack list and where its tarballs download from, instead of the bundled list
    /// with no network.
    packs: Option<(
        Vec<td_host::packs::Curated>,
        Box<dyn td_host::catalog::Fetcher>,
    )>,
    ports: Ports,
}
impl Default for StartOptions {
    fn default() -> Self {
        Self {
            limits: crate::Limits::default(),
            asset_store_bytes: crate::store::ASSET_STORE_BYTES,
            release: false,
            grants: None,
            desktop: None,
            catalog: Box::new(NoCatalogNetwork),
            packs: None,
            ports: Ports::Mock,
        }
    }
}
pub struct TestCompanion {
    ctx: Ctx,
    companion: Option<crate::companion::Companion>,
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
    pub async fn start_with_asset_quota(asset_store_bytes: u64) -> Self {
        Self::start_router(StartOptions {
            asset_store_bytes,
            ..Default::default()
        })
        .await
    }
    /// Starts with other connection and time limits, so limit tests run in milliseconds.
    pub async fn start_with(limits: crate::Limits) -> Self {
        Self::start_router(StartOptions {
            limits,
            ..Default::default()
        })
        .await
    }
    pub async fn start_with_catalog(
        limits: crate::Limits,
        fetcher: Box<dyn td_host::catalog::Fetcher>,
    ) -> Self {
        Self::start_router(StartOptions {
            limits,
            catalog: fetcher,
            ..Default::default()
        })
        .await
    }
    /// Starts offering `curated` as the Icon pack list, downloaded through `fetcher`.
    pub async fn start_with_packs(
        curated: Vec<td_host::packs::Curated>,
        fetcher: Box<dyn td_host::catalog::Fetcher>,
    ) -> Self {
        Self::start_router(StartOptions {
            packs: Some((curated, fetcher)),
            ..Default::default()
        })
        .await
    }
    pub async fn start_release_router() -> Self {
        Self::start_router(StartOptions {
            release: true,
            ..Default::default()
        })
        .await
    }
    /// Starts on mock ports that the Companion treats as real ones, as a Companion without
    /// `--mock-devices` does, with or without `--test-real-device-console`.
    pub async fn start_on_real_ports(device_console: bool) -> Self {
        Self::start_router(StartOptions {
            ports: Ports::Real { device_console },
            ..Default::default()
        })
        .await
    }
    /// Starts with grants kept in `grants` instead of a file.
    pub async fn start_with_grants(grants: Arc<dyn GrantStore>) -> Self {
        Self::start_router(StartOptions {
            grants: Some(grants),
            ..Default::default()
        })
        .await
    }
    async fn start_router(options: StartOptions) -> Self {
        let StartOptions {
            limits,
            asset_store_bytes,
            release,
            grants,
            desktop,
            catalog: fetcher,
            packs,
            ports: mode,
        } = options;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let data = tempfile::tempdir().unwrap();
        let path = data.path().to_owned();
        let mut handle = tokio::task::spawn_blocking(move || {
            let grants = grants
                .unwrap_or_else(|| Arc::new(FileGrants::new(path.join("allowed-actions.json"))));
            crate::companion::Handle::new(true, &path, asset_store_bytes, grants)
        })
        .await
        .unwrap();
        handle.catalog = std::sync::Arc::new(td_host::catalog::Catalog::new(
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
        handle.packs = std::sync::Arc::new(td_host::packs::Packs::with_list(
            data.path().join("packs"),
            pack_fetcher,
            curated,
        ));
        if let Some(desktop) = desktop {
            handle.desktop = desktop;
        }
        let mut config = Config::mock(Some(data.path().join("devices.json")));
        let (ports, input) = config.mock.clone().unwrap();
        if let Ports::Real { .. } = mode {
            config.mock = None;
        }
        let companion = crate::companion::Companion::launch(handle.clone(), config);
        let mut ctx = Ctx::new(handle, port, None, limits);
        if let Ports::Real { device_console } = mode {
            ctx.real_device_console = device_console;
        }
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
            companion: Some(companion),
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
        *self = Self::start_router(StartOptions {
            desktop: Some(desktop),
            ..Default::default()
        })
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
    /// Plugs in a mock Device with the radio build's v2 Firmware, which can join Wi-Fi and has a Bluetooth
    /// keyboard, and waits until it's connected.
    pub async fn plug_radio_device(&self, id: &str) -> DeviceId {
        self.plug(id, BoardKind::DeviceRadio).await
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
    /// needed but not plugged in. While `announce` is on it announces itself through mock
    /// discovery, with the record a real Device sends: its id only while it is pairing. Mock
    /// Devices report the id `020000000001`, so use that serial for USB and Wi-Fi to merge.
    pub fn wifi_device(&self, serial: &str, announce: bool) -> td_link::net::fake::FakeNetDevice {
        self.wifi_board(serial, BoardKind::Device, announce)
    }
    /// [`wifi_device`](Self::wifi_device) with the radio build behind it, as `plug_radio_device` plugs it in.
    pub fn wifi_radio_device(
        &self,
        serial: &str,
        announce: bool,
    ) -> td_link::net::fake::FakeNetDevice {
        self.wifi_board(serial, BoardKind::DeviceRadio, announce)
    }
    fn wifi_board(
        &self,
        serial: &str,
        kind: BoardKind,
        announce: bool,
    ) -> td_link::net::fake::FakeNetDevice {
        let deck = self.ports.board(serial, kind);
        let fake = td_link::net::fake::FakeNetDevice::start(deck);
        if announce {
            self.wifi_network().announce(&fake);
        }
        fake
    }
    /// The in-memory keychain of Pairing keys.
    pub fn wifi_keys(&self) -> crate::devices::MemoryKeys {
        self.ctx.devices.wifi_mock().expect("mock Wi-Fi").0.clone()
    }
    /// Makes the Companion think a paired Device is at `address`, as an unseen DHCP move would.
    pub fn wifi_move(&self, device: &DeviceId, address: std::net::SocketAddr) {
        self.ctx.devices.wifi_move(device, address);
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
        tokio::time::timeout(within, async {
            let _ = (&mut self.task).await;
            if let Some(companion) = self.companion.take() {
                companion.shutdown().await;
            }
        })
        .await
        .is_ok()
    }
    /// The Brandfetch settings and provenance, to point it at a local server.
    pub fn brandfetch(&self) -> &crate::features::brandfetch::Brandfetch {
        &self.ctx.brandfetch
    }
    /// This Companion's data directory, a fresh temporary one.
    pub fn data_dir(&self) -> &std::path::Path {
        self._data.path()
    }
    /// Runs one Brandfetch refresh as if the wall clock read `now`, in Unix seconds.
    pub async fn refresh_brandfetch(&self, now: u64) -> crate::features::brandfetch::RefreshReport {
        let ctx = self.ctx.clone();
        tokio::task::spawn_blocking(move || ctx.brandfetch.refresh(&ctx.assets, now))
            .await
            .unwrap()
    }
    /// Starts the Companion's Brandfetch refresh loop, checking every `tick` by the clock that
    /// [`Self::clock`] moves. It stops when the handle is aborted.
    pub fn start_brandfetch_refresh(&self, tick: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(crate::features::brandfetch::refresh_loop(
            self.ctx.companion_handle(),
            tick,
        ))
    }
    /// Runs one 60 s maintenance step now.
    pub async fn maintain(&self) {
        // Sweep sessions before yielding to pack eviction, as the process maintenance did.
        // Otherwise an expired socket can close first and its row is pruned by the sweep.
        assert!(crate::api::maintain(&self.ctx).await);
        self.ctx.companion_handle().maintain().await;
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
            self.ctx.web_events.publish_frame(
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
