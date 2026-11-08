use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use td_host::{
    integrations::{Environment, Registry, secrets::*},
    sources::{CliProxy, Fetch, HttpFetch, KeyStore, Source},
};

#[derive(Default, Clone)]
struct Recorder(Arc<Mutex<Vec<(String, String)>>>);
impl Fetch for Recorder {
    fn get(&self, url: &str, bearer: &str) -> Result<String, String> {
        self.0.lock().unwrap().push((url.into(), bearer.into()));
        Ok("[]".into())
    }
    fn post_json(&self, _: &str, _: &str, _: &Value) -> Result<String, String> {
        panic!("empty fixture has no upstream calls")
    }
}
fn registry(recorder: Recorder, file: Option<std::path::PathBuf>) -> Registry {
    Registry::new(Environment {
        media_provider: Some(std::sync::Arc::new(
            td_host::integrations::media::fake::FakeMedia::default(),
        )),
        volume: Some(td_host::integrations::volume::fake::FakeVolume::default().factory()),
        secrets: Arc::new(MemorySecrets::default()),
        config_file: file,
        fetch: Arc::new(move || Box::new(recorder.clone())),
    })
    .unwrap()
}
fn config(url: &str) -> Value {
    json!({"enabled":true,"url":url,"interval_s":1})
}

#[test]
fn refresh_during_proxy_poll_survives_apply_and_respects_spacing() {
    let calls = Recorder::default();
    let mut r = registry(calls.clone(), None);
    let i = r.get("cliproxyapi").unwrap();
    i.configure(config("https://hub.example")).unwrap();
    i.secret_put("management_key", "fixture-key").unwrap();
    let poll = i.prepare_poll().unwrap();
    i.request_refresh();
    i.apply_poll(poll());
    assert_eq!(calls.0.lock().unwrap().len(), 1);
    assert!(i.prepare_poll().is_none());
    std::thread::sleep(td_host::sources::PROXY_REFRESH_FLOOR);
    let poll = i
        .prepare_poll()
        .expect("the queued refresh must survive applying the first poll");
    i.apply_poll(poll());
    assert_eq!(calls.0.lock().unwrap().len(), 2);
}

#[test]
fn registry_schema_presets_config_and_summaries_never_contain_secret_values() {
    let root = std::env::temp_dir().join(format!("td-integration-test-{}", std::process::id()));
    let file = root.join("sources.json");
    let mut r = registry(Recorder::default(), Some(file.clone()));
    let summaries = r.list();
    assert!(summaries.iter().any(|s| s.id == "this_pc"));
    let s = &summaries[0];
    assert_eq!(s.id, "cliproxyapi");
    assert!(
        s.config_schema
            .iter()
            .any(|f| f.name == "management_key" && f.secret)
    );
    assert!(s.presets.iter().any(|p| p.id == "five_hour_pct"));
    for summary in summaries.into_iter().filter(|s| s.id == "cliproxyapi") {
        let i = r.get(&summary.id).unwrap();
        i.configure(config("https://hub.example")).unwrap();
        i.secret_put("management_key", "fixture-private-value")
            .unwrap();
        assert!(
            !serde_json::to_string(&i.view().unwrap())
                .unwrap()
                .contains("fixture-private-value")
        );
        let mut bad = config("https://hub.example");
        bad["management_key"] = "fixture-private-value".into();
        assert!(i.configure(bad).is_err());
        assert!(
            !std::fs::read_to_string(&file)
                .unwrap()
                .contains("fixture-private-value")
        );
        assert_eq!(i.view().unwrap().config["interval_s"], 60);
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn saved_unknown_integration_config_and_credentials_do_not_block_registry_startup() {
    let removed_id = "retired_integration";
    let root = std::env::temp_dir().join(format!("td-unknown-integration-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("sources.json");
    let saved = root.join(format!("{removed_id}.json"));
    let store = Arc::new(MemorySecrets::default());
    for field in ["token_id", "token_secret"] {
        store
            .put(
                removed_id,
                field,
                Secret {
                    origin: "https://retired.example".into(),
                    value: "fixture-unused-secret".into(),
                },
            )
            .unwrap();
    }
    let mut initial = registry(Recorder::default(), Some(file.clone()));
    initial
        .get("cliproxyapi")
        .unwrap()
        .configure(json!({"enabled":false,"url":"https://hub.example","interval_s":120}))
        .unwrap();
    for contents in [
        json!({"enabled":true,"url":"https://retired.example","interval_s":15,"tls_pin":""})
            .to_string(),
        "not valid JSON".into(),
    ] {
        std::fs::write(&saved, &contents).unwrap();
        let calls = Recorder::default();
        let fetch = calls.clone();
        let mut loaded = Registry::new(Environment {
            media_provider: Some(std::sync::Arc::new(
                td_host::integrations::media::fake::FakeMedia::default(),
            )),
            volume: Some(td_host::integrations::volume::fake::FakeVolume::default().factory()),
            secrets: store.clone(),
            config_file: Some(file.clone()),
            fetch: Arc::new(move || Box::new(fetch.clone())),
        })
        .unwrap();
        assert!(matches!(
            loaded.get(removed_id),
            Err(td_host::integrations::Error::NotFound)
        ));
        assert!(loaded.get("this_pc").unwrap().view().is_ok());
        let view = loaded.get("cliproxyapi").unwrap().view().unwrap();
        assert_eq!(view.config["url"], "https://hub.example");
        assert!(!view.secrets["management_key"].set);
        loaded.tick();
        assert!(calls.0.lock().unwrap().is_empty());
        let summaries = serde_json::to_string(&loaded.list()).unwrap();
        assert!(!summaries.contains(removed_id));
        assert!(!summaries.contains("fixture-unused-secret"));
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), contents);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scheme_host_and_effective_port_clear_secrets_but_path_does_not() {
    for next in [
        "http://hub.example",
        "https://other.example",
        "https://hub.example:8443",
    ] {
        let calls = Recorder::default();
        let mut r = registry(calls.clone(), None);
        let i = r.get("cliproxyapi").unwrap();
        i.configure(config("https://hub.example")).unwrap();
        i.secret_put("management_key", "fixture-key").unwrap();
        let same = i.configure(config("https://hub.example:443/path")).unwrap();
        assert!(same.secrets["management_key"].set);
        i.refresh();
        assert_eq!(calls.0.lock().unwrap().len(), 1);
        let changed = i.configure(config(next)).unwrap();
        assert!(!changed.secrets["management_key"].set);
        i.tick();
        assert_eq!(calls.0.lock().unwrap().len(), 1);
    }
}
#[test]
fn typed_private_http_is_allowed_but_public_http_never_reaches_even_fake_fetch() {
    for (url, allowed) in [
        ("http://localhost:8317", true),
        ("http://127.0.0.1", true),
        ("http://10.0.0.1", true),
        ("http://172.16.0.1", true),
        ("http://192.168.1.1", true),
        ("http://[::1]", true),
        ("http://[fd00::1]", true),
        ("http://public.example", false),
        ("http://private-resolving.example", false),
        ("http://192.0.2.1", false),
        ("https://public.example", true),
    ] {
        let calls = Recorder::default();
        let mut r = registry(calls.clone(), None);
        let i = r.get("cliproxyapi").unwrap();
        i.configure(config(url)).unwrap();
        i.secret_put("management_key", "fixture-key").unwrap();
        i.refresh();
        assert_eq!(!calls.0.lock().unwrap().is_empty(), allowed, "{url}");
    }
}
#[test]
fn old_pollers_cannot_obtain_a_new_origins_secret() {
    let keys = Arc::new(
        BoundKeys::new(Arc::new(MemorySecrets::default()), "https://old.example").unwrap(),
    );
    keys.set("old-fixture").unwrap();
    let calls = Recorder::default();
    let mut old = CliProxy::new("https://old.example", keys.clone(), Box::new(calls.clone()));
    keys.rebind("https://new.example").unwrap();
    keys.set("new-fixture").unwrap();
    old.poll();
    assert!(calls.0.lock().unwrap().is_empty());
    assert_eq!(
        keys.get_for("https://new.example").unwrap().as_deref(),
        Some("new-fixture")
    );
}
#[test]
fn real_http_client_does_not_follow_a_cross_origin_redirect() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let token = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = format!("/redirect-{token}");
    let dest_path = format!("/dest-{token}");
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}{path}", redirect.local_addr().unwrap());
    let target = destination.local_addr().unwrap();
    let expected = format!("GET {path} HTTP/1.1\r\n");
    let redirect_to = dest_path.clone();
    let server = std::thread::spawn(move || {
        for accepted in redirect.incoming() {
            let Ok(mut stream) = accepted else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut bytes = [0; 4096];
            let mut filled = 0;
            while filled < bytes.len() {
                match stream.read(&mut bytes[filled..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        filled += n;
                        if bytes[..filled].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            if !bytes[..filled].starts_with(expected.as_bytes()) {
                continue;
            }
            assert!(String::from_utf8_lossy(&bytes[..filled]).contains("fixture-key"));
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://{target}{redirect_to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            return;
        }
        panic!("redirect request not received");
    });
    assert!(HttpFetch::new().get(&url, "fixture-key").is_err());
    server.join().unwrap();
    while let Ok((mut stream, _)) = destination.accept() {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        let mut bytes = [0; 1024];
        let n = stream.read(&mut bytes).unwrap_or(0);
        assert!(!String::from_utf8_lossy(&bytes[..n]).contains(&dest_path));
    }
}

#[derive(Default)]
struct LegacyStore {
    store: MemorySecrets,
    legacy: Mutex<Option<String>>,
    fail: Mutex<bool>,
}
impl SecretStore for LegacyStore {
    fn get(&self, i: &str, f: &str) -> Result<Option<Secret>, String> {
        self.store.get(i, f)
    }
    fn put(&self, i: &str, f: &str, s: Secret) -> Result<(), String> {
        if *self.fail.lock().unwrap() {
            Err("unavailable".into())
        } else {
            self.store.put(i, f, s)
        }
    }
    fn delete(&self, i: &str, f: &str) -> Result<(), String> {
        self.store.delete(i, f)
    }
    fn legacy(&self) -> Result<Option<String>, String> {
        Ok(self.legacy.lock().unwrap().clone())
    }
    fn delete_legacy(&self) -> Result<(), String> {
        *self.legacy.lock().unwrap() = None;
        Ok(())
    }
}
#[test]
fn migration_preserves_legacy_until_the_bound_copy_is_saved() {
    let store = Arc::new(LegacyStore::default());
    *store.legacy.lock().unwrap() = Some("legacy-fixture".into());
    *store.fail.lock().unwrap() = true;
    let keys = BoundKeys::new(store.clone(), "https://hub.example").unwrap();
    assert!(keys.get().is_err());
    assert!(store.legacy.lock().unwrap().is_some());
    *store.fail.lock().unwrap() = false;
    assert_eq!(keys.get().unwrap().as_deref(), Some("legacy-fixture"));
    assert!(store.legacy.lock().unwrap().is_none());
    assert_eq!(
        store
            .get("cliproxyapi", "management_key")
            .unwrap()
            .unwrap()
            .origin,
        "https://hub.example"
    );
}

#[test]
fn the_contract_publishes_existing_readings_and_device_delivery() {
    let mut r = registry(Recorder::default(), None);
    let i = r.get("cliproxyapi").unwrap();
    i.configure(config("https://hub.example")).unwrap();
    i.secret_put("management_key", "fixture-key").unwrap();
    i.refresh();
    assert!(
        i.readings()
            .iter()
            .any(|r| r.field == "status" && r.value == "none")
    );
    i.delivered(td_host::integrations::Delivery {
        device: "mock0001".into(),
        sent: 3,
        error: None,
    });
    i.delivered(td_host::integrations::Delivery {
        device: "mock0002".into(),
        sent: 0,
        error: Some("offline".into()),
    });
    assert_eq!(i.summary().status.delivery[0].sent, 3);
    assert_eq!(
        i.summary().status.delivery[1].error.as_deref(),
        Some("offline")
    );
    i.secret_delete("management_key").unwrap();
    assert!(i.readings().is_empty());
}

#[cfg(windows)]
#[test]
fn windows_vault_round_trip_uses_an_isolated_fixture_entry() {
    let store = OsSecrets;
    let id = format!(
        "test-78-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    store
        .put(
            &id,
            "fixture",
            Secret {
                origin: "https://hub.example".into(),
                value: "fixture-only".into(),
            },
        )
        .unwrap();
    let read = store.get(&id, "fixture");
    store.delete(&id, "fixture").unwrap();
    let read = read.unwrap().unwrap();
    assert_eq!(read.origin, "https://hub.example");
    assert_eq!(read.value, "fixture-only");
    assert!(store.get(&id, "fixture").unwrap().is_none());
}

#[test]
fn invalid_saved_origin_is_repairable_and_never_sends_a_legacy_secret() {
    let store = Arc::new(LegacyStore::default());
    *store.legacy.lock().unwrap() = Some("legacy-fixture".into());
    let keys = BoundKeys::from_config(store.clone(), "not a URL");
    assert!(keys.get().is_err());
    keys.rebind("https://hub.example").unwrap();
    assert!(keys.get().unwrap().is_none());
    keys.set("replacement-fixture").unwrap();
    assert_eq!(keys.get().unwrap().as_deref(), Some("replacement-fixture"));
}

#[test]
fn reconfiguring_does_not_bypass_the_manual_refresh_floor() {
    let calls = Recorder::default();
    let mut r = registry(calls.clone(), None);
    let i = r.get("cliproxyapi").unwrap();
    i.configure(config("https://hub.example")).unwrap();
    i.secret_put("management_key", "fixture-key").unwrap();
    i.refresh();
    i.configure(config("https://hub.example/path")).unwrap();
    i.refresh();
    assert_eq!(calls.0.lock().unwrap().len(), 1);
}

#[test]
fn repeated_configure_and_tick_preserve_the_poll_deadline() {
    let calls = Recorder::default();
    let mut r = registry(calls.clone(), None);
    let i = r.get("cliproxyapi").unwrap();
    i.configure(config("https://hub.example")).unwrap();
    i.secret_put("management_key", "fixture-key").unwrap();
    for _ in 0..5 {
        i.configure(config("https://hub.example")).unwrap();
        i.tick();
    }
    assert_eq!(calls.0.lock().unwrap().len(), 1);
    std::thread::sleep(td_host::sources::PROXY_REFRESH_FLOOR);
    i.configure(config("https://hub.example")).unwrap();
    i.tick();
    assert_eq!(calls.0.lock().unwrap().len(), 1);
    i.refresh();
    assert_eq!(calls.0.lock().unwrap().len(), 2);
}
#[test]
fn secret_writes_ticks_and_refresh_share_request_spacing() {
    let calls = Recorder::default();
    let mut r = registry(calls.clone(), None);
    let i = r.get("cliproxyapi").unwrap();
    i.configure(config("https://hub.example")).unwrap();
    for _ in 0..5 {
        i.secret_put("management_key", "fixture-key").unwrap();
        i.tick();
        i.configure(config("https://hub.example/path")).unwrap();
        i.refresh();
    }
    assert_eq!(calls.0.lock().unwrap().len(), 1);
}
#[test]
fn secret_stores_reject_ambiguous_target_components() {
    for store in [&MemorySecrets::default() as &dyn SecretStore, &OsSecrets] {
        for (i, f) in [("a:b", "c"), ("a", "b:c"), ("", "c"), ("a", "bad field")] {
            assert!(store.get(i, f).is_err());
            assert!(
                store
                    .put(
                        i,
                        f,
                        Secret {
                            origin: "https://hub.example".into(),
                            value: "fixture".into()
                        }
                    )
                    .is_err()
            );
            assert!(store.delete(i, f).is_err());
        }
    }
}

#[test]
fn detail_starts_empty_and_polling_changes_the_revision() {
    let mut r = registry(Recorder::default(), None);
    let i = r.get("cliproxyapi").unwrap();
    i.configure(config("https://hub.example")).unwrap();
    i.secret_put("management_key", "fixture-key").unwrap();
    let before = serde_json::to_value(i.view().unwrap()).unwrap();
    assert_eq!(before["detail"], Value::Null);
    i.refresh();
    let after = serde_json::to_value(i.view().unwrap()).unwrap();
    assert_eq!(after["detail"]["accounts"], json!([]));
    assert!(after["detail"]["polled_ms"].as_u64().unwrap() > 0);
    assert_ne!(before["summary"]["revision"], after["summary"]["revision"]);
    i.secret_delete("management_key").unwrap();
    assert_eq!(
        serde_json::to_value(i.view().unwrap()).unwrap()["detail"],
        Value::Null
    );
}

fn registry_with(store: Arc<MemorySecrets>, file: std::path::PathBuf) -> Registry {
    Registry::new(Environment {
        media_provider: Some(std::sync::Arc::new(
            td_host::integrations::media::fake::FakeMedia::default(),
        )),
        volume: Some(td_host::integrations::volume::fake::FakeVolume::default().factory()),
        secrets: store,
        config_file: Some(file),
        fetch: Arc::new(|| Box::new(Recorder::default())),
    })
    .unwrap()
}

#[test]
fn set_enabled_changes_only_enabled_keeps_every_secret_and_is_saved() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sources.json");
    let store = Arc::new(MemorySecrets::default());
    let mut r = registry_with(store.clone(), file.clone());
    let proxy = r.get("cliproxyapi").unwrap();
    proxy.configure(config("https://hub.example")).unwrap();
    proxy.secret_put("management_key", "fixture-key").unwrap();
    let custom = r.get("custom").unwrap();
    custom
        .configure(json!({"enabled":true,"sources":[{"id":"temperature","name":"Temperature","interval_s":60,"kind":"number","input":{"type":"http","url":"https://hub.example/metrics","value_path":"$.value","headers":{"Authorization":"auth"}}}]}))
        .unwrap();
    custom.secret_put("auth", "fixture-header").unwrap();
    let ids: Vec<String> = r.list().into_iter().map(|s| s.id).collect();
    for id in ["cliproxyapi", "custom", "media", "this_pc", "volume"] {
        assert!(ids.iter().any(|i| i == id), "{id}");
    }
    for id in &ids {
        let i = r.get(id).unwrap();
        let before = i.view().unwrap();
        assert!(before.summary.enabled, "{id}");
        for enabled in [false, false, true] {
            let after = i.set_enabled(enabled).unwrap();
            let mut expected = before.config.clone();
            expected["enabled"] = enabled.into();
            assert_eq!(after.summary.enabled, enabled, "{id}");
            assert_eq!(after.config, expected, "{id}");
            assert_eq!(
                serde_json::to_value(&after.secrets).unwrap(),
                serde_json::to_value(&before.secrets).unwrap(),
                "{id}"
            );
        }
        i.set_enabled(false).unwrap();
    }
    assert_eq!(
        store
            .get("cliproxyapi", "management_key")
            .unwrap()
            .unwrap()
            .value,
        "fixture-key"
    );
    assert_eq!(
        store.get("custom", "auth").unwrap().unwrap().value,
        "fixture-header"
    );
    let mut reloaded = registry_with(store, file);
    for id in &ids {
        let view = reloaded.get(id).unwrap().view().unwrap();
        assert!(!view.summary.enabled, "{id}");
    }
    let proxy = reloaded.get("cliproxyapi").unwrap().view().unwrap();
    assert_eq!(
        proxy.config,
        json!({"enabled":false,"url":"https://hub.example","interval_s":60})
    );
    assert!(proxy.secrets["management_key"].set);
    assert!(reloaded.get("custom").unwrap().view().unwrap().secrets["auth"].set);
}

#[test]
fn cliproxy_set_enabled_keeps_the_saved_url_as_written_and_its_key() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sources.json");
    std::fs::write(
        &file,
        json!({"cliproxy":false,"cliproxy_url":"https://hub.example/","cliproxy_interval_s":300})
            .to_string(),
    )
    .unwrap();
    let store = Arc::new(MemorySecrets::default());
    store
        .put(
            "cliproxyapi",
            "management_key",
            Secret {
                origin: "https://hub.example".into(),
                value: "fixture-key".into(),
            },
        )
        .unwrap();
    let mut r = registry_with(store.clone(), file.clone());
    let i = r.get("cliproxyapi").unwrap();
    for enabled in [true, false] {
        let view = i.set_enabled(enabled).unwrap();
        assert_eq!(
            view.config,
            json!({"enabled":enabled,"url":"https://hub.example/","interval_s":300})
        );
        assert!(view.secrets["management_key"].set);
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved["cliproxy"], enabled);
        assert_eq!(saved["cliproxy_url"], "https://hub.example/");
        assert_eq!(saved["cliproxy_interval_s"], 300);
    }
    assert_eq!(
        store
            .get("cliproxyapi", "management_key")
            .unwrap()
            .unwrap()
            .value,
        "fixture-key"
    );
}
