#[cfg(test)]
mod delivery_tests {
    use crate::integrations::*;

    fn sent(service: &Service) -> Vec<(String, u64)> {
        let list = service.list().unwrap();
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
        let mut changes = events.subscribe();
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

        // Even an unsuccessful user write reconciles delivery, refreshes the cache
        // and publishes the change before returning its error.
        assert!(service.secret_put("this_pc", "missing", "value").is_err());
        let cached = service
            .summaries()
            .into_iter()
            .find(|s| s.id == "this_pc")
            .unwrap();
        assert_eq!(cached.status.delivery[0].sent, 4);
        let mut published = false;
        while let Ok(crate::events::Change::IntegrationStatus(event)) = changes.try_recv() {
            if event.0.id == "this_pc" {
                assert_eq!(event.0.status.delivery[0].sent, 4);
                published = true;
            }
        }
        assert!(published);
        assert_eq!(sent(&service), [("mock0001".to_owned(), 4)]);
    }
}

#[cfg(test)]
mod control_tests {
    use crate::devices::{Config, DeviceManager, LinkState};
    use crate::integrations::*;
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
    use crate::integrations::*;
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
            .with_registry(|r| Ok(r.get("custom")?.live()))
            .unwrap()
            .into_iter()
            .map(|sample| sample.value)
            .collect()
    }
    fn refresh_custom(service: &Service) {
        service.refresh("custom").unwrap();
    }
    #[test]
    fn slow_poll_does_not_block_user_changes() {
        let (service, _) = start();
        type Operation = fn(&Service) -> Result<(), Error>;
        let operations: [Operation; 7] = [
            |s| s.list().map(|_| ()),
            |s| s.view("custom").map(|_| ()),
            |s| s.refresh("custom"),
            |s| s.secret_put("cliproxyapi", "management_key", "fixture-private-value"),
            |s| s.secret_delete("cliproxyapi", "management_key"),
            |s| s.configure("custom", config(true, "new")).map(|_| ()),
            |s| s.set_enabled("custom", false).map(|_| ()),
        ];
        for operation in operations {
            let at = Instant::now();
            operation(&service).unwrap();
            assert!(
                at.elapsed() < Duration::from_millis(200),
                "user change took {:?}",
                at.elapsed()
            );
        }
    }
    #[test]
    fn setup_cancel_and_save_then_enable_do_not_wait_for_another_integrations_poll() {
        let (service, calls) = start_with(SLOWER);
        let at = Instant::now();
        service.setup("cliproxyapi", true).unwrap().cancel();
        let mut setup = service.setup("cliproxyapi", false).unwrap();
        let settings = serde_json::json!({"url":"https://hub.example","interval_s":120});
        let secrets = BTreeMap::from([("management_key".into(), "fixture-private-value".into())]);
        assert!(!setup.save(settings, secrets).unwrap().summary.enabled);
        assert!(
            service
                .set_enabled("cliproxyapi", true)
                .unwrap()
                .summary
                .enabled
        );
        assert!(at.elapsed() < Duration::from_millis(200));
        assert_eq!(calls.finished(), 0);
        assert!(this_pc_advances(&service, 2));
        assert!(service.view("cliproxyapi").unwrap().summary.enabled);
    }

    #[test]
    fn stale_form_save_while_polling_keeps_the_switch_and_discards_the_old_result() {
        let (service, calls) = start();
        let mut form = service.view("custom").unwrap().config;
        form.as_object_mut().unwrap().remove("enabled");
        service.set_enabled("custom", false).unwrap();
        let saved = service.configure("custom", form).unwrap();
        assert!(!saved.summary.enabled);
        assert!(wait_until(Duration::from_secs(5), || calls.finished() > 0));
        assert!(this_pc_advances(&service, 2));
        assert!(!service.view("custom").unwrap().summary.enabled);
        assert!(custom_values(&service).is_empty());
    }

    #[test]
    fn setup_save_then_enable_discards_that_integrations_in_flight_result() {
        let (service, calls) = start();
        service.set_enabled("custom", false).unwrap();
        let mut setup = service.setup("custom", true).unwrap();
        let view = setup.save(config(false, "new"), BTreeMap::new()).unwrap();
        assert!(view.summary.enabled);
        assert!(wait_until(Duration::from_secs(5), || calls.finished() > 0));
        assert!(this_pc_advances(&service, 2));
        assert!(service.view("custom").unwrap().summary.enabled);
        assert!(!custom_values(&service).contains(&td_core::live::LiveValue::Number(1.0)));
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
        let view = service.configure("custom", config(enabled, "new")).unwrap();
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
        let summary = service.view("custom").unwrap().summary;
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
