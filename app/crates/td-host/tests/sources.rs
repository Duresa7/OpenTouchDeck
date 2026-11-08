use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use td_core::{DashboardRow, Layout, Request};
use td_host::sources::cliproxy::{
    API_CALL, AUTH_FILES, CLAUDE_USAGE, CODEX_USAGE, ProxyInfo, insecure_http, parse_accounts,
    readings as proxy_readings,
};
use td_host::sources::{
    CliProxy, Config, Fetch, Hub, KeyStore, MemoryKeys, Reading, Scheduler, Sink, Source, key_hint,
};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn dev_layout() -> Layout {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../schema/examples/dev.json");
    Layout::parse(&fs::read(path).unwrap()).unwrap()
}

fn dev_layout_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../schema/examples/dev.json")
        .display()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "td-host-{name}-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn by_field(readings: &[Reading]) -> HashMap<String, Reading> {
    readings
        .iter()
        .filter(|reading| reading.account.is_empty())
        .map(|reading| (reading.field.clone(), reading.clone()))
        .collect()
}

#[test]
fn the_auth_files_fixture_yields_one_account_per_entry_with_recent_10m_summed() {
    let accounts = parse_accounts(&fixture("auth-files.json")).unwrap();
    let shown: Vec<_> = accounts
        .iter()
        .map(|a| {
            (
                a.provider.as_str(),
                a.email.as_str(),
                a.status.as_str(),
                a.success,
                a.failed,
                a.recent_10m,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("claude", "ada@example.com", "active", 120, 3, 5),
            ("codex", "grace@example.com", "active", 80, 0, 12),
            ("gemini", "linus@example.com", "error", 5, 9, 0),
        ]
    );
    let readings = proxy_readings(&accounts, 1, false);
    let get = |account: &str, field: &str| {
        readings
            .iter()
            .find(|r| r.account == account && r.field == field)
            .map(|r| (r.value.clone(), r.bar))
            .unwrap()
    };
    assert_eq!(get("", "status"), ("error".into(), -1.0));
    assert_eq!(get("", "success"), ("205".into(), -1.0));
    assert_eq!(get("", "failed"), ("12".into(), -1.0));
    assert_eq!(get("", "recent_10m"), ("17".into(), -1.0));
    assert_eq!(
        get("codex:grace@example.com", "recent_10m"),
        ("12".into(), 1.0)
    );
    assert_eq!(
        get("claude:ada@example.com", "recent_10m"),
        ("5".into(), 5.0 / 12.0)
    );
    assert_eq!(
        get("gemini:linus@example.com", "recent_10m"),
        ("0".into(), 0.0)
    );
    assert_eq!(readings.len(), 4 + 3 * 4 + 1);
}

#[test]
fn a_bare_array_and_sparse_entries_still_parse() {
    let accounts =
        parse_accounts(r#"[{"name":"k.json","disabled":true},{"email":"alice@example.com"}]"#)
            .unwrap();
    assert_eq!(accounts[0].email, "k.json");
    assert_eq!(accounts[0].status, "disabled");
    assert_eq!(accounts[1].status, "unknown");
    assert_eq!(accounts[1].recent_10m, 0);
    assert!(parse_accounts("<html>").is_err());
    assert!(
        proxy_readings(&[], 1, false)
            .iter()
            .any(|r| r.field == "status" && r.value == "none")
    );
}

#[derive(Clone, Default)]
struct FakeFetch {
    steps: Arc<Mutex<Vec<Result<String, String>>>>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
    posts: Arc<Mutex<Vec<(String, String, Value)>>>,
    usage: Arc<Mutex<HashMap<String, Result<String, String>>>>,
}

impl FakeFetch {
    fn script(&self, steps: Vec<Result<String, String>>) {
        let mut queue = self.steps.lock().unwrap();
        *queue = steps;
        queue.reverse();
    }
}

impl Fetch for FakeFetch {
    fn get(&self, url: &str, bearer: &str) -> Result<String, String> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_owned(), bearer.to_owned()));
        self.steps
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(|| Err("script ran out".into()))
    }

    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<String, String> {
        self.posts
            .lock()
            .unwrap()
            .push((url.to_owned(), bearer.to_owned(), body.clone()));
        // The Go handler accepts all three spellings, but only singular `header`.
        let auth_index = ["auth_index", "authIndex", "AuthIndex"]
            .into_iter()
            .filter_map(|field| body[field].as_str())
            .map(str::trim)
            .find(|index| !index.is_empty())
            .ok_or("HTTP 400")?;
        if body["header"]["Authorization"] != "Bearer $TOKEN$" {
            return Ok(serde_json::json!({"status_code":401,"body":""}).to_string());
        }
        self.usage.lock().unwrap().get(auth_index).cloned()
            .unwrap_or_else(|| Ok(serde_json::json!({"status_code":200,"body":"{\"five_hour\":null,\"seven_day\":null,\"rate_limit\":null}"}).to_string()))
    }
}

fn proxy_with(fetch: &FakeFetch, key: Option<&str>) -> (CliProxy, Arc<MemoryKeys>) {
    let keys = Arc::new(MemoryKeys::default());
    if let Some(key) = key {
        keys.set(key).unwrap();
    }
    let proxy = CliProxy::new(
        "http://127.0.0.1:8317/",
        keys.clone(),
        Box::new(fetch.clone()),
    );
    (proxy, keys)
}

#[test]
fn three_failed_polls_mark_the_readings_stale_and_the_next_good_poll_clears_it() {
    let fetch = FakeFetch::default();
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key-1234"));
    fetch.script(vec![
        Ok(fixture("auth-files.json")),
        Err("connection refused".into()),
        Err("connection refused".into()),
        Err("connection refused".into()),
        Ok(fixture("auth-files.json")),
    ]);
    let now = SystemTime::now();
    let stale = |readings: &[Reading]| readings.iter().all(|reading| reading.stale);
    let none_stale = |readings: &[Reading]| readings.iter().all(|reading| !reading.stale);
    let first = proxy.poll_at(now);
    assert!(!first.is_empty() && none_stale(&first));
    assert!(none_stale(&proxy.poll_at(now)));
    assert!(none_stale(&proxy.poll_at(now)));
    let third = proxy.poll_at(now);
    assert_eq!(third.len(), first.len());
    assert!(stale(&third));
    let td_host::sources::Info::Proxy(info) = proxy.info();
    assert_eq!((info.ok, info.fails, info.stale), (Some(false), 3, true));
    assert_eq!(info.error.as_deref(), Some("connection refused"));
    let back = proxy.poll_at(now);
    assert!(none_stale(&back));
}

#[test]
fn auth_file_refreshes_use_bearer_authorization_and_redact_the_key() {
    let fetch = FakeFetch::default();
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key-1234"));
    fetch.script(vec![
        Ok(fixture("auth-files.json")),
        Ok(fixture("auth-files.json")),
    ]);
    proxy.poll_at(SystemTime::now());
    proxy.refresh();
    proxy.refresh();
    // Provider POSTs have separate coverage; this log records auth-file GETs.
    let calls = fetch.calls.lock().unwrap().clone();
    assert_eq!(
        calls.len(),
        2,
        "a second refresh inside the floor does not poll again"
    );
    for (url, bearer) in &calls {
        assert_eq!(url, &format!("http://127.0.0.1:8317{AUTH_FILES}"));
        assert_eq!(bearer, "fixture-key-1234");
        assert!(!url.contains("usage-queue"));
    }
    let td_host::sources::Info::Proxy(info) = proxy.info();
    let shown = serde_json::to_string(&info).unwrap();
    assert!(shown.contains("…1234") && !shown.contains("fixture-key"));
    assert_eq!(key_hint("ab"), "Set");
}

#[test]
fn no_key_is_a_failed_poll_that_makes_no_request() {
    let fetch = FakeFetch::default();
    let (mut proxy, _) = proxy_with(&fetch, None);
    assert!(proxy.poll_at(SystemTime::now()).is_empty());
    assert!(fetch.calls.lock().unwrap().is_empty());
    let td_host::sources::Info::Proxy(info) = proxy.info();
    assert_eq!(info.error.as_deref(), Some("no management key is set"));
}

fn proxy_info(proxy: &CliProxy) -> ProxyInfo {
    let td_host::sources::Info::Proxy(info) = proxy.info();
    info
}

fn api_reply(body: &str) -> Result<String, String> {
    Ok(serde_json::json!({"status_code": 200, "header": {}, "body": body}).to_string())
}

fn usage_fetch() -> FakeFetch {
    let fetch = FakeFetch::default();
    fetch.script(vec![Ok(fixture("auth-files-usage.json"))]);
    fetch.usage.lock().unwrap().extend([
        (
            "codex-alice".to_owned(),
            api_reply(&fixture("usage-codex.json")),
        ),
        (
            "claude-alice".to_owned(),
            api_reply(&fixture("usage-claude.json")),
        ),
    ]);
    fetch
}

#[test]
fn usage_windows_keys_resets_and_aggregate_maxima_are_reported() {
    let fetch = usage_fetch();
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
    let readings = proxy.poll_at(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    let info = proxy_info(&proxy);
    assert_eq!(info.accounts.len(), 4);
    assert_eq!(info.accounts[0].key, "codex:alice@example.com");
    assert_eq!(info.accounts[1].key, "claude:alice@example.com");
    assert_eq!(info.accounts[0].plan.as_deref(), Some("pro"));
    assert_eq!(info.accounts[1].plan.as_deref(), Some("Claude"));
    assert_eq!(info.accounts[3].status, "disabled");
    let get = |account: &str, field: &str| {
        readings
            .iter()
            .find(|r| r.account == account && r.field == field)
            .unwrap()
    };
    assert_eq!(get("codex:alice@example.com", "five_hour_pct").value, "42%");
    assert_eq!(get("codex:alice@example.com", "five_hour_pct").bar, 0.42);
    for who in ["codex:alice@example.com", "claude:alice@example.com"] {
        assert_eq!(get(who, "five_hour_reset").value, "2h 10m");
        assert_eq!(get(who, "seven_day_reset").value, "3d 4h");
        assert_eq!(get(who, "seven_day_reset").bar, -1.0);
    }
    assert_eq!(get("", "five_hour_pct").value, "42%");
    assert_eq!(get("", "seven_day_pct").value, "61%");
    assert_eq!(get("", "success").value, "205");
    assert_eq!(get("", "recent_10m").value, "17");
    assert!(
        info.accounts[2..]
            .iter()
            .all(|a| a.five_hour.is_none() && a.seven_day.is_none() && a.usage_error.is_none())
    );
    assert_eq!(fetch.calls.lock().unwrap().len(), 1);
    let posts = fetch.posts.lock().unwrap();
    assert_eq!(
        posts.len(),
        2,
        "unsupported and disabled accounts make no upstream request"
    );
    assert!(posts.iter().all(
        |(url, key, _)| url == &format!("http://127.0.0.1:8317{API_CALL}") && key == "fixture-key"
    ));
    let codex = &posts
        .iter()
        .find(|(_, _, b)| b["url"] == CODEX_USAGE)
        .unwrap()
        .2;
    let claude = &posts
        .iter()
        .find(|(_, _, b)| b["url"] == CLAUDE_USAGE)
        .unwrap()
        .2;
    // Matches the served management center and its upstream header definitions:
    // router-for-me/Cli-Proxy-API-Management-Center@50c3b9fb, src/utils/quota/constants.ts.
    assert_eq!(
        codex,
        &serde_json::json!({
            "authIndex":"codex-alice", "method":"GET", "url":CODEX_USAGE,
            "header":{"Authorization":"Bearer $TOKEN$", "Content-Type":"application/json", "User-Agent":"codex_cli_rs/0.76.0 (Debian 13.0.0; x86_64) WindowsTerminal", "Chatgpt-Account-Id":"fixture-account-alice"}
        })
    );
    assert_eq!(
        claude,
        &serde_json::json!({
            "authIndex":"claude-alice", "method":"GET", "url":CLAUDE_USAGE,
            "header":{"Authorization":"Bearer $TOKEN$", "Content-Type":"application/json", "anthropic-beta":"oauth-2025-04-20"}
        })
    );
    let serialized = serde_json::to_string(&info).unwrap();
    assert!(
        !serialized.contains("fixture-account-alice")
            && !serialized.contains("id_token")
            && !serialized.contains("auth_index")
    );
}

#[test]
fn codex_windows_follow_durations_or_default_positions_and_clamp() {
    for durations in [true, false] {
        let fetch = FakeFetch::default();
        fetch.script(vec![Ok(r#"[{"provider":"codex","email":"alice@example.com","auth_index":"fixture-index","id_token":{"chatgpt_plan_type":"plus"}}]"#.to_owned())]);
        let mut body = serde_json::json!({"rate_limit": {
            "primary_window":{"used_percent":142,"reset_at":1699999999},
            "secondary_window":{"used_percent":-4,"reset_at":null}
        }});
        if durations {
            body["rate_limit"]["primary_window"]["limit_window_seconds"] = 604800.into();
            body["rate_limit"]["secondary_window"]["limit_window_seconds"] = 86400.into();
        }
        fetch
            .usage
            .lock()
            .unwrap()
            .insert("fixture-index".to_owned(), api_reply(&body.to_string()));
        let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
        proxy.poll_at(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let info = proxy_info(&proxy);
        let a = &info.accounts[0];
        assert_eq!(a.plan.as_deref(), Some("plus"));
        assert_eq!(
            a.five_hour.as_ref().unwrap().pct,
            if durations { 0.0 } else { 100.0 }
        );
        assert_eq!(
            a.seven_day.as_ref().unwrap().pct,
            if durations { 100.0 } else { 0.0 }
        );
        let (past, absent) = if durations {
            (&a.seven_day, &a.five_hour)
        } else {
            (&a.five_hour, &a.seven_day)
        };
        assert_eq!(past.as_ref().unwrap().reset_in.as_deref(), Some("now"));
        assert_eq!(absent.as_ref().unwrap().reset_in, None);
        let posts = fetch.posts.lock().unwrap();
        assert_eq!(posts[0].2["header"].as_object().unwrap().len(), 3);
        assert!(posts[0].2["header"].get("Chatgpt-Account-Id").is_none());
    }
}

#[test]
fn claude_null_windows_and_unsupported_providers_do_not_invent_usage() {
    let fetch = usage_fetch();
    fetch.usage.lock().unwrap().insert(
        "claude-alice".to_owned(),
        api_reply(r#"{"five_hour":null,"seven_day":{"utilization":120,"resets_at":null}}"#),
    );
    fetch.usage.lock().unwrap().insert(
        "codex-alice".to_owned(),
        api_reply(r#"{"rate_limit":null}"#),
    );
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
    let readings = proxy.poll();
    assert!(
        !readings
            .iter()
            .any(|r| r.field == "five_hour_pct" || r.field == "five_hour_reset")
    );
    assert_eq!(by_field(&readings)["seven_day_pct"].value, "100%");
    let info = proxy_info(&proxy);
    assert_eq!(info.accounts[0].plan.as_deref(), Some("plus"));
    assert!(info.accounts.iter().all(|a| a.usage_error.is_none()));
}

#[test]
fn every_provider_failure_is_local_and_errors_never_echo_sensitive_input() {
    let cases = [
        (
            Ok(serde_json::json!({"status_code":401,"body":"fixture-provider-token"}).to_string()),
            "provider refused (HTTP 401)",
        ),
        (
            api_reply("{fixture-provider-token"),
            "malformed usage reply",
        ),
        (
            Err("fixture-key fixture-provider-token fixture-account-id".to_owned()),
            "hub request failed",
        ),
        (Ok("fixture-key".to_owned()), "malformed api-call reply"),
        (
            api_reply(
                r#"{"five_hour":{"utilization":20},"seven_day":{"utilization":10,"resets_at":"fixture-key"}}"#,
            ),
            "malformed reset time",
        ),
    ];
    for (reply, expected) in cases {
        let fetch = usage_fetch();
        fetch
            .usage
            .lock()
            .unwrap()
            .insert("claude-alice".to_owned(), reply);
        let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
        let readings = proxy.poll();
        let info = proxy_info(&proxy);
        assert_eq!((info.ok, info.fails, info.stale), (Some(true), 0, false));
        assert_eq!(info.accounts[1].usage_error.as_deref(), Some(expected));
        assert!(info.accounts[1].five_hour.is_none() && info.accounts[1].seven_day.is_none());
        assert_eq!(by_field(&readings)["seven_day_pct"].value, "10%");
        let shown = serde_json::to_string(&info).unwrap();
        for secret in [
            "fixture-key",
            "fixture-provider-token",
            "fixture-account-id",
        ] {
            assert!(!shown.contains(secret));
        }
    }
    let fetch = FakeFetch::default();
    fetch.script(vec![
        Err("fixture-key in transport error".to_owned()),
        Ok("{fixture-key".to_owned()),
    ]);
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
    proxy.poll();
    assert_eq!(
        proxy_info(&proxy).error.as_deref(),
        Some("hub request failed")
    );
    proxy.poll();
    assert_eq!(
        proxy_info(&proxy).error.as_deref(),
        Some("not an auth-files reply")
    );
}

#[test]
fn usage_dashboard_uses_provider_local_part_and_the_higher_window() {
    let fetch = usage_fetch();
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
    let mut scheduler = Scheduler::from_layout(&dev_layout());
    scheduler.update("cliproxyapi", proxy.poll());
    let sent = scheduler.due(Instant::now());
    let rows = sent
        .iter()
        .find_map(|r| match r {
            Request::DashboardSet { rows, .. } => Some(rows),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        rows[0],
        DashboardRow {
            label: "codex alice".to_owned(),
            value: "5h 42% 7d 10%".to_owned(),
            bar: 0.42
        }
    );
    assert_eq!(
        rows[1],
        DashboardRow {
            label: "claude alice".to_owned(),
            value: "5h 24% 7d 61%".to_owned(),
            bar: 0.61
        }
    );
    assert_eq!(
        rows[2],
        DashboardRow {
            label: "gemini-cli linus".to_owned(),
            value: "error".to_owned(),
            bar: 0.0
        }
    );
    assert_eq!(rows[3].value, "disabled");
}

#[test]
fn a_failed_usage_window_clears_the_previous_widget_value_and_recovers() {
    let layout = Layout::parse(br#"{"schema":1,"name":"Usage","orientation":"landscape","pages":[{"id":"p","title":"P","kind":"grid","rows":1,"cols":1,"tiles":[{"id":"usage","label":"Usage","row":0,"col":0,"rowSpan":1,"colSpan":1,"icon":null,"action":{"type":"widget","source":"cliproxyapi","field":"five_hour_pct","account":"codex:alice@example.com"}}]}]}"#).unwrap();
    let fetch = usage_fetch();
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
    let mut scheduler = Scheduler::from_layout(&layout);
    let start = Instant::now();
    for (n, expected) in ["42%", "unavailable", "42%"].into_iter().enumerate() {
        fetch.script(vec![Ok(fixture("auth-files-usage.json"))]);
        fetch.usage.lock().unwrap().insert(
            "codex-alice".to_owned(),
            if n == 1 {
                Err("failed".to_owned())
            } else {
                api_reply(&fixture("usage-codex.json"))
            },
        );
        scheduler.update("cliproxyapi", proxy.poll());
        assert_eq!(
            tile_value(
                &scheduler.due(start + Duration::from_secs(n as u64 * 2)),
                "usage"
            )
            .as_deref(),
            Some(expected)
        );
    }
    fetch.script(vec![Ok("[]".to_owned())]);
    scheduler.update("cliproxyapi", proxy.poll());
    assert_eq!(
        tile_value(&scheduler.due(start + Duration::from_secs(6)), "usage").as_deref(),
        Some("unavailable")
    );
}

#[test]
fn proxy_interval_defaults_to_two_minutes_and_has_a_one_minute_floor() {
    assert_eq!(Config::default().interval(), Duration::from_secs(120));
    for seconds in [0, 1, 59, 60, 120, 600] {
        assert_eq!(
            Config {
                cliproxy_interval_s: seconds,
                ..Config::default()
            }
            .interval(),
            Duration::from_secs(seconds.max(60))
        );
    }
}

#[test]
fn provider_calls_run_in_parallel_with_at_most_four_in_flight() {
    struct ConcurrentFetch {
        active: AtomicUsize,
        peak: AtomicUsize,
        calls: AtomicUsize,
        barrier: Barrier,
    }
    struct SharedFetch(Arc<ConcurrentFetch>);
    impl Fetch for SharedFetch {
        fn get(&self, url: &str, _: &str) -> Result<String, String> {
            assert!(url.ends_with(AUTH_FILES));
            Ok(serde_json::json!({"files": (0..8).map(|i| serde_json::json!({
                "provider":"codex", "email":format!("account{i}@example.com"), "auth_index":format!("fixture-{i}")
            })).collect::<Vec<_>>()}).to_string())
        }
        fn post_json(&self, url: &str, _: &str, body: &Value) -> Result<String, String> {
            assert!(url.ends_with(API_CALL));
            assert_eq!(body["url"], CODEX_USAGE);
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.0.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.0.peak.fetch_max(active, Ordering::SeqCst);
            self.0.barrier.wait();
            self.0.active.fetch_sub(1, Ordering::SeqCst);
            api_reply(&fixture("usage-codex.json"))
        }
    }
    let fetch = Arc::new(ConcurrentFetch {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        barrier: Barrier::new(4),
    });
    let keys = Arc::new(MemoryKeys::default());
    keys.set("fixture-key").unwrap();
    let mut proxy = CliProxy::new(
        "http://127.0.0.1",
        keys,
        Box::new(SharedFetch(fetch.clone())),
    );
    proxy.poll();
    assert_eq!(fetch.peak.load(Ordering::SeqCst), 4);
    assert_eq!(fetch.calls.load(Ordering::SeqCst), 8);
    assert_eq!(proxy_info(&proxy).accounts.len(), 8);
}

#[test]
fn sparse_accounts_fall_back_to_label_name_and_id_and_skip_unknown_providers() {
    let fetch = FakeFetch::default();
    fetch.script(vec![Ok(r#"[{"provider":"codex","label":"label"},{"provider":"claude","name":"name"},{"provider":"antigravity","id":"id"}]"#.to_owned())]);
    let (mut proxy, _) = proxy_with(&fetch, Some("fixture-key"));
    proxy.poll();
    let info = proxy_info(&proxy);
    assert_eq!(
        info.accounts
            .iter()
            .map(|a| a.key.as_str())
            .collect::<Vec<_>>(),
        ["codex:label", "claude:name", "antigravity:id"]
    );
    assert_eq!(
        info.accounts[0].usage_error.as_deref(),
        Some("missing auth index")
    );
    assert_eq!(
        info.accounts[1].usage_error.as_deref(),
        Some("missing auth index")
    );
    assert!(info.accounts[2].usage_error.is_none());
    assert!(fetch.posts.lock().unwrap().is_empty());
}

type RequestLog = Arc<Mutex<Vec<(Instant, String, Option<String>)>>>;

struct FakeServer {
    url: String,
    requests: RequestLog,
    stop: Arc<AtomicBool>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(key: &'static str, body: String) -> FakeServer {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let prefix = format!("/hub-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}{prefix}", listener.local_addr().unwrap());
    let requests: RequestLog = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (log, halt) = (requests.clone(), stop.clone());
    thread::spawn(move || {
        while !halt.load(Ordering::Relaxed) {
            let Ok((stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            answer(stream, &prefix, key, &body, &log);
        }
    });
    FakeServer {
        url,
        requests,
        stop,
    }
}

fn answer(
    mut stream: TcpStream,
    prefix: &str,
    key: &str,
    body: &str,
    log: &Mutex<Vec<(Instant, String, Option<String>)>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let Some((method, target_and_version)) = request_line.trim().split_once(' ') else {
        return;
    };
    let Some(stripped) = target_and_version.strip_prefix(prefix) else {
        return;
    };
    let normalized = format!("{method} {stripped}");
    let mut auth = None;
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("authorization")
        {
            auth = Some(value.trim().to_owned());
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            let Ok(parsed) = value.trim().parse() else {
                return;
            };
            length = parsed;
        }
    }
    let mut request_body = vec![0; length];
    if reader.read_exact(&mut request_body).is_err() {
        return;
    }
    log.lock()
        .unwrap()
        .push((Instant::now(), normalized.clone(), auth.clone()));
    let api = serde_json::json!({"status_code": 200, "body": "{\"five_hour\":null,\"seven_day\":null,\"rate_limit\":null}"}).to_string();
    let (status, payload) = if auth.as_deref() != Some(&format!("Bearer {key}")) {
        ("401 Unauthorized", "{\"error\":\"missing management key\"}")
    } else if normalized.starts_with(&format!("GET {AUTH_FILES} ")) {
        ("200 OK", body)
    } else if normalized.starts_with(&format!("POST {API_CALL} ")) {
        ("200 OK", api.as_str())
    } else {
        ("404 Not Found", "{}")
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let _ = stream.flush();
}

#[test]
fn http_fetch_sends_one_authorized_get_and_reports_a_refusal() {
    let server = serve("fixture-good-key", fixture("auth-files.json"));
    let keys = Arc::new(MemoryKeys::default());
    keys.set("fixture-good-key").unwrap();
    let mut proxy = CliProxy::new(
        &server.url,
        keys.clone(),
        Box::new(td_host::sources::HttpFetch::new()),
    );
    assert_eq!(proxy.poll_at(SystemTime::now()).len(), 17);
    {
        let seen = server.requests.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].1, format!("GET {AUTH_FILES} HTTP/1.1"));
        assert_eq!(seen[0].2.as_deref(), Some("Bearer fixture-good-key"));
        assert!(seen[1..].iter().all(|(_, line, auth)| line
            == &format!("POST {API_CALL} HTTP/1.1")
            && auth.as_deref() == Some("Bearer fixture-good-key")));
    }
    keys.set("fixture-wrong-key").unwrap();
    proxy.poll_at(SystemTime::now());
    let td_host::sources::Info::Proxy(info) = proxy.info();
    assert_eq!(info.error.as_deref(), Some("HTTP 401"));
}

#[test]
fn https_urls_are_accepted_and_http_warnings_only_apply_off_loopback() {
    let fetch = FakeFetch::default();
    fetch.script(vec![Ok(fixture("auth-files.json"))]);
    let keys = Arc::new(MemoryKeys::default());
    keys.set("fixture-key").unwrap();
    let mut proxy = CliProxy::new("https://127.0.0.1", keys, Box::new(fetch.clone()));
    proxy.poll();
    assert_eq!(proxy_info(&proxy).ok, Some(true));
    assert_eq!(
        fetch.calls.lock().unwrap()[0].0,
        format!("https://127.0.0.1{AUTH_FILES}")
    );
    for url in [
        "http://127.0.0.1:8317",
        "http://127.1.2.3",
        "http://[::1]",
        "http://[::ffff:127.0.0.1]",
        "http://localhost",
        "http://localhost.",
        "https://example.com",
    ] {
        assert!(!insecure_http(url), "{url}");
    }
    assert!(insecure_http("http://example.com"));
    assert!(insecure_http("http://localhost.example.com"));
}

fn widget(value: &str, bar: f64, stale: bool) -> Reading {
    Reading::new("cliproxyapi", "", "five_hour_pct", value, bar, 1, stale)
}

fn context_sets(requests: &[Request]) -> Vec<(String, bool)> {
    requests
        .iter()
        .filter_map(|request| match request {
            Request::WidgetSet {
                tile, value, stale, ..
            } if tile == "five_hour" => Some((value.clone(), *stale)),
            _ => None,
        })
        .collect()
}

#[test]
fn dev_json_binds_four_widget_tiles_and_one_dashboard() {
    let scheduler = Scheduler::from_layout(&dev_layout());
    assert_eq!(scheduler.counts(), (4, 1));
    assert_eq!(scheduler.source_of_tile("proxy"), Some("cliproxyapi"));
    assert_eq!(scheduler.source_of_tile("five_hour"), Some("cliproxyapi"));
    assert_eq!(scheduler.source_of_tile("f5"), None);
}

#[test]
fn the_scheduler_sends_once_per_change_and_respects_the_two_second_floor() {
    let mut scheduler = Scheduler::from_layout(&dev_layout());
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    assert!(scheduler.due(at(0)).is_empty(), "nothing is known yet");

    scheduler.update("cliproxyapi", vec![widget("42%", 0.42, false)]);
    let first = scheduler.due(at(0));
    assert_eq!(
        first,
        [Request::WidgetSet {
            tile: "five_hour".into(),
            value: "42%".into(),
            caption: "5h usage".into(),
            bar: 0.42,
            stale: false
        }]
    );
    assert!(scheduler.due(at(1)).is_empty(), "no change, no send");
    assert!(
        scheduler.due(at(5000)).is_empty(),
        "no change, no send, however late"
    );
    assert_eq!(scheduler.pending_in(at(5000)), None);

    scheduler.update("cliproxyapi", vec![widget("43%", 0.43, false)]);
    assert_eq!(
        context_sets(&scheduler.due(at(5100))),
        [("43%".to_owned(), false)]
    );
    assert!(scheduler.due(at(5101)).is_empty());
}

#[test]
fn a_change_inside_the_floor_waits_and_a_burst_sends_only_the_last_value() {
    let mut scheduler = Scheduler::from_layout(&dev_layout());
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    scheduler.update("cliproxyapi", vec![widget("42%", 0.42, false)]);
    let mut sent = scheduler.due(at(0));
    scheduler.update("cliproxyapi", vec![widget("43%", 0.43, false)]);
    sent.extend(scheduler.due(at(500)));
    scheduler.update("cliproxyapi", vec![widget("44%", 0.44, false)]);
    sent.extend(scheduler.due(at(900)));
    scheduler.update("cliproxyapi", vec![widget("45%", 0.45, false)]);
    sent.extend(scheduler.due(at(1500)));
    assert_eq!(context_sets(&sent), [("42%".to_owned(), false)]);
    assert_eq!(
        scheduler.pending_in(at(1500)),
        Some(Duration::from_millis(500))
    );
    sent.extend(scheduler.due(at(1999)));
    assert_eq!(context_sets(&sent).len(), 1);
    sent.extend(scheduler.due(at(2000)));
    assert_eq!(
        context_sets(&sent),
        [("42%".to_owned(), false), ("45%".to_owned(), false)]
    );
    assert_eq!(scheduler.pending_in(at(2000)), None);
}

#[test]
fn a_stale_flip_is_sent_and_so_is_its_recovery() {
    let mut scheduler = Scheduler::from_layout(&dev_layout());
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    scheduler.update("cliproxyapi", vec![widget("42%", 0.42, false)]);
    let mut sent = scheduler.due(at(0));
    scheduler.update("cliproxyapi", vec![widget("42%", 0.42, true)]);
    sent.extend(scheduler.due(at(2000)));
    scheduler.update("cliproxyapi", vec![widget("42%", 0.42, false)]);
    sent.extend(scheduler.due(at(4000)));
    assert_eq!(
        context_sets(&sent),
        [
            ("42%".to_owned(), false),
            ("42%".to_owned(), true),
            ("42%".to_owned(), false)
        ]
    );
}

#[test]
fn a_resync_and_a_failed_send_both_come_due_again() {
    let mut scheduler = Scheduler::from_layout(&dev_layout());
    let now = Instant::now();
    scheduler.update("cliproxyapi", vec![widget("42%", 0.42, false)]);
    let sent = scheduler.due(now);
    assert_eq!(sent.len(), 1);
    scheduler.forget(&sent[0]);
    assert_eq!(
        scheduler.due(now).len(),
        1,
        "a failed send is retried at once"
    );
    scheduler.resync();
    assert_eq!(
        scheduler.due(now).len(),
        1,
        "a new connection gets everything again"
    );
}

#[test]
fn a_dashboard_page_gets_one_row_per_account_scaled_to_the_busiest() {
    let mut scheduler = Scheduler::from_layout(&dev_layout());
    let accounts = parse_accounts(&fixture("auth-files.json")).unwrap();
    scheduler.update("cliproxyapi", proxy_readings(&accounts, 1, false));
    let sent = scheduler.due(Instant::now());
    let dashboard = sent
        .iter()
        .find_map(|request| match request {
            Request::DashboardSet { page, rows } => Some((page.clone(), rows.clone())),
            _ => None,
        })
        .expect("a dashboard.set");
    assert_eq!(dashboard.0, "accounts");
    assert_eq!(
        dashboard.1,
        [
            DashboardRow {
                label: "claude ada".into(),
                value: "active".into(),
                bar: 5.0 / 12.0
            },
            DashboardRow {
                label: "codex grace".into(),
                value: "active".into(),
                bar: 1.0
            },
            DashboardRow {
                label: "gemini linus".into(),
                value: "error".into(),
                bar: 0.0
            },
        ]
    );
    let proxy = sent
        .iter()
        .find_map(|request| match request {
            Request::WidgetSet { tile, value, .. } if tile == "proxy" => Some(value.clone()),
            _ => None,
        })
        .expect("a widget.set for the proxy tile");
    assert_eq!(proxy, "error", "the empty account is the worst status");

    scheduler.update("cliproxyapi", proxy_readings(&accounts, 1, true));
    let stale = scheduler.due(Instant::now() + Duration::from_secs(2));
    let Some(Request::DashboardSet { rows, .. }) = stale
        .iter()
        .find(|r| matches!(r, Request::DashboardSet { .. }))
    else {
        panic!("the stale flip sends the page again");
    };
    assert!(
        rows.iter()
            .all(|row| row.value == "stale" && row.bar == -1.0)
    );
}

#[test]
fn a_dashboard_keeps_eight_rows_and_cuts_long_text() {
    let mut scheduler = Scheduler::from_layout(&dev_layout());
    let accounts: Vec<_> = (0..11)
        .map(|n| format!(r#"{{"email":"account-number-{n}@example.com","status":"active"}}"#))
        .collect();
    let body = format!("[{}]", accounts.join(","));
    scheduler.update(
        "cliproxyapi",
        proxy_readings(&parse_accounts(&body).unwrap(), 1, false),
    );
    let sent = scheduler.due(Instant::now());
    let Some(Request::DashboardSet { rows, .. }) = sent
        .iter()
        .find(|r| matches!(r, Request::DashboardSet { .. }))
    else {
        panic!("a dashboard.set");
    };
    assert_eq!(rows.len(), 8);
    assert!(rows.iter().all(|row| row.label.chars().count() <= 16));
}

#[test]
fn a_widget_only_matches_its_account() {
    let layout = Layout::parse(
        br#"{"schema":1,"name":"t","orientation":"landscape","pages":[{"id":"p","title":"P","kind":"grid","rows":1,"cols":2,"tiles":[
        {"id":"grace","label":"Grace","row":0,"col":0,"rowSpan":1,"colSpan":1,"icon":null,"action":{"type":"widget","source":"cliproxyapi","field":"recent_10m","account":"codex:grace@example.com"}},
        {"id":"ctx","label":"Ctx","row":0,"col":1,"rowSpan":1,"colSpan":1,"icon":null,"action":{"type":"widget","source":"cliproxyapi","field":"recent_10m","account":"anyone"}}]}]}"#,
    )
    .unwrap();
    let mut scheduler = Scheduler::from_layout(&layout);
    let accounts = parse_accounts(&fixture("auth-files.json")).unwrap();
    scheduler.update("cliproxyapi", proxy_readings(&accounts, 1, false));
    let sent = scheduler.due(Instant::now());
    assert!(sent.contains(&Request::WidgetSet {
        tile: "grace".into(),
        value: "12".into(),
        caption: "Grace".into(),
        bar: 1.0,
        stale: false
    }));
    assert!(
        !sent
            .iter()
            .any(|r| matches!(r, Request::WidgetSet { tile, .. } if tile == "ctx"))
    );
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(Instant, Request)>>);

impl Sink for Recorder {
    fn send(&self, request: &Request) -> Result<Option<String>, String> {
        self.0
            .lock()
            .unwrap()
            .push((Instant::now(), request.clone()));
        Ok(None)
    }
}

struct Flaky {
    fail: AtomicBool,
    taken: Mutex<Vec<Request>>,
}

impl Sink for Flaky {
    fn send(&self, request: &Request) -> Result<Option<String>, String> {
        if self.fail.load(Ordering::Relaxed) {
            return Err("link closed".into());
        }
        self.taken.lock().unwrap().push(request.clone());
        Ok(None)
    }
}

impl Recorder {
    fn sent(&self) -> Vec<Request> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(_, request)| request.clone())
            .collect()
    }

    fn wait_for(
        &self,
        what: &str,
        within: Duration,
        done: impl Fn(&[Request]) -> bool,
    ) -> Duration {
        let started = Instant::now();
        while started.elapsed() < within {
            if done(&self.sent()) {
                return started.elapsed();
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("{what} did not happen within {within:?}: {:?}", self.sent());
    }
}

fn tile_value(requests: &[Request], tile: &str) -> Option<String> {
    requests.iter().rev().find_map(|request| match request {
        Request::WidgetSet { tile: t, value, .. } if t == tile => Some(value.clone()),
        _ => None,
    })
}

fn proxy_config(url: &str) -> Config {
    Config {
        cliproxy: true,
        cliproxy_url: url.to_owned(),
        layout_path: Some(dev_layout_path()),
        ..Config::default()
    }
}

#[test]
fn a_failed_send_loses_nothing_and_every_tile_arrives_once_the_link_is_back() {
    let server = serve("fixture-proxy-key", fixture("auth-files.json"));
    let keys = Arc::new(MemoryKeys::default());
    keys.set("fixture-proxy-key").unwrap();
    let hub = Hub::start(proxy_config(&server.url), keys);
    let sink = Arc::new(Flaky {
        fail: AtomicBool::new(true),
        taken: Mutex::new(Vec::new()),
    });
    hub.set_sink(Some(sink.clone()));
    let started = Instant::now();
    while hub.status().send_error.is_none() && started.elapsed() < Duration::from_secs(2) {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(hub.status().send_error.as_deref(), Some("link closed"));
    assert!(sink.taken.lock().unwrap().is_empty());
    sink.fail.store(false, Ordering::Relaxed);
    let started = Instant::now();
    while sink.taken.lock().unwrap().len() < 5 && started.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(300));
    let taken = sink.taken.lock().unwrap().clone();
    let mut tiles: Vec<_> = taken
        .iter()
        .filter_map(|r| match r {
            Request::WidgetSet { tile, .. } => Some(tile.clone()),
            _ => None,
        })
        .collect();
    tiles.sort();
    assert_eq!(tiles, ["claude_5h", "five_hour", "proxy", "seven_day"]);
    assert_eq!(hub.status().send_error, None);
    hub.stop();
}

#[test]
fn a_new_sink_gets_every_tile_again() {
    let server = serve("fixture-proxy-key", fixture("auth-files.json"));
    let keys = Arc::new(MemoryKeys::default());
    keys.set("fixture-proxy-key").unwrap();
    let hub = Hub::start(proxy_config(&server.url), keys);
    let first = Arc::new(Recorder::default());
    hub.set_sink(Some(first.clone()));
    first.wait_for(
        "four tiles and a dashboard",
        Duration::from_secs(2),
        |sent| sent.len() == 5,
    );
    hub.set_sink(None);
    let second = Arc::new(Recorder::default());
    hub.set_sink(Some(second.clone()));
    second.wait_for(
        "four tiles and a dashboard again",
        Duration::from_secs(2),
        |sent| sent.len() == 5,
    );
    hub.stop();
}

#[test]
fn a_tap_on_a_proxy_tile_polls_once_at_once_and_the_dashboard_lists_each_account() {
    let server = serve("fixture-proxy-key", fixture("auth-files.json"));
    let keys = Arc::new(MemoryKeys::default());
    keys.set("fixture-proxy-key").unwrap();
    let hub = Hub::start(
        Config {
            cliproxy: true,
            cliproxy_url: server.url.clone(),
            layout_path: Some(dev_layout_path()),
            ..Config::default()
        },
        keys,
    );
    let sink = Arc::new(Recorder::default());
    hub.set_sink(Some(sink.clone()));
    sink.wait_for("the first dashboard", Duration::from_secs(3), |sent| {
        sent.iter()
            .any(|r| matches!(r, Request::DashboardSet { rows, .. } if rows.len() == 3))
    });
    assert_eq!(tile_value(&sink.sent(), "proxy").as_deref(), Some("error"));
    assert_eq!(server.requests.lock().unwrap().len(), 3);

    thread::sleep(Duration::from_millis(300));
    let pressed = Instant::now();
    hub.pressed("f5");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        server.requests.lock().unwrap().len(),
        3,
        "a tap elsewhere polls nothing"
    );
    hub.pressed("proxy");
    let deadline = Instant::now() + Duration::from_secs(1);
    while server.requests.lock().unwrap().len() < 6 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 6, "the tap polls");
    assert!(requests[3].0 > pressed);
    for _ in 0..10 {
        hub.pressed("proxy");
        hub.refresh("cliproxyapi");
    }
    thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        server.requests.lock().unwrap().len(),
        6,
        "and ignores extra refreshes inside 15 s"
    );
    hub.stop();
}

#[test]
fn configure_swaps_the_layout_and_reports_a_bad_one() {
    let dir = scratch("hub-config");
    let hub = Hub::start(Config::default(), Arc::new(MemoryKeys::default()));
    hub.configure(Config {
        layout_path: Some(dir.join("missing.json").display().to_string()),
        ..Config::default()
    });
    let started = Instant::now();
    while hub.status().layout.error.is_none() && started.elapsed() < Duration::from_secs(2) {
        thread::sleep(Duration::from_millis(10));
    }
    let status = hub.status();
    assert!(status.layout.error.is_some() && status.layout.widgets == 0);
    hub.stop();
}

#[test]
fn the_config_file_round_trips_and_never_holds_the_key() {
    let dir = scratch("config");
    let path = dir.join("touchdeck").join("sources.json");
    let config = Config {
        cliproxy: true,
        cliproxy_interval_s: 2,
        ..Config::default()
    };
    config.save(&path).unwrap();
    assert_eq!(Config::load(&path), config);
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.to_lowercase().contains("key"));
    fs::write(&path, "not json").unwrap();
    assert_eq!(Config::load(&path), Config::default());
}

#[test]
fn old_source_config_ignores_removed_keys_and_keeps_proxy_settings() {
    let dir = scratch("old-config");
    let path = dir.join("sources.json");
    fs::write(
        &path,
        r#"{
        "claude_status": true, "claude_dir": "old-state", "claude_settings": "old-settings.json",
        "cliproxy": true, "cliproxy_url": "http://localhost:8318", "cliproxy_interval_s": 240,
        "layout_path": "layout.json"
    }"#,
    )
    .unwrap();
    let expected = Config {
        cliproxy: true,
        cliproxy_url: "http://localhost:8318".to_owned(),
        cliproxy_interval_s: 240,
        layout_path: Some("layout.json".to_owned()),
    };
    assert_eq!(Config::load(&path), expected);
    expected.save(&path).unwrap();
    let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved.as_object().unwrap().len(), 4);
    fs::remove_dir_all(dir).unwrap();
}
