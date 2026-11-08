//! Usage limits through CLIProxyAPI. Each poll lists auth files, then asks the hub to
//! GET usage from two fixed provider URLs. Never consumes usage-queue or resets quota.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::{Host, Url};

use super::{
    CLIPROXYAPI, FAILS_TO_STALE, Info, KeyStore, Reading, RefreshGate, Source, key_hint, unix_ms,
};

pub const AUTH_FILES: &str = "/v0/management/auth-files";
pub const API_CALL: &str = "/v0/management/api-call";
pub const CLAUDE_USAGE: &str = "https://api.anthropic.com/api/oauth/usage";
pub const CODEX_USAGE: &str = "https://chatgpt.com/backend-api/wham/usage";
const TIMEOUT: Duration = Duration::from_secs(15);
const CONCURRENCY: usize = 4;
const TLS_ERROR: &str = "TLS certificate verification failed; for a private CA, add it to the Windows certificate store";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    pub pct: f64,
    /// Unix seconds, when the provider supplies a reset time.
    pub resets_at: Option<i64>,
    /// Remaining time at this poll, not a client-side countdown.
    pub reset_in: Option<String>,
}

/// Public account data. Auth indexes and id_token values never leave the poll.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Account {
    pub key: String,
    pub provider: String,
    pub email: String,
    pub status: String,
    pub success: u64,
    pub failed: u64,
    /// Requests in the newest `recent_requests` bucket, which spans 10 minutes.
    pub recent_10m: u64,
    pub plan: Option<String>,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub usage_error: Option<String>,
    pub insecure_http: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Bucket {
    success: u64,
    failed: u64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct IdToken {
    chatgpt_account_id: Option<String>,
    plan_type: Option<String>,
    chatgpt_plan_type: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry {
    id: String,
    auth_index: String,
    name: String,
    label: String,
    email: String,
    provider: String,
    r#type: String,
    status: String,
    disabled: bool,
    unavailable: bool,
    success: u64,
    failed: u64,
    recent_requests: Vec<Bucket>,
    id_token: Option<IdToken>,
}

impl Entry {
    fn account(&self) -> Account {
        let pick = |names: [&str; 4]| {
            names
                .into_iter()
                .find(|s| !s.trim().is_empty())
                .unwrap_or("")
                .to_owned()
        };
        let provider = pick([&self.provider, &self.r#type, &self.label, &self.name]);
        let email = pick([&self.email, &self.label, &self.name, &self.id]);
        let status = if self.disabled {
            "disabled"
        } else if !self.status.trim().is_empty() {
            &self.status
        } else if self.unavailable {
            "unavailable"
        } else {
            "unknown"
        };
        let plan = match provider.as_str() {
            "claude" => Some("Claude".to_owned()),
            "codex" => self.id_token.as_ref().and_then(|id| {
                id.plan_type
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .or(id.chatgpt_plan_type.as_ref())
                    .cloned()
            }),
            _ => None,
        };
        Account {
            key: format!("{provider}:{email}"),
            provider,
            email,
            status: status.to_owned(),
            success: self.success,
            failed: self.failed,
            recent_10m: self
                .recent_requests
                .last()
                .map_or(0, |b| b.success.saturating_add(b.failed)),
            plan,
            five_hour: None,
            seven_day: None,
            usage_error: None,
            insecure_http: false,
        }
    }

    /// The only allowlist for upstream requests. No URL from auth-files is used.
    fn usage_request(&self, provider: &str) -> Option<Value> {
        let (url, mut header) = match provider {
            "claude" => (
                CLAUDE_USAGE,
                json!({
                    "Authorization": "Bearer $TOKEN$", "Content-Type": "application/json",
                    "anthropic-beta": "oauth-2025-04-20"
                }),
            ),
            "codex" => (
                CODEX_USAGE,
                json!({
                    "Authorization": "Bearer $TOKEN$", "Content-Type": "application/json",
                    "User-Agent": "codex_cli_rs/0.76.0 (Debian 13.0.0; x86_64) WindowsTerminal"
                }),
            ),
            _ => return None,
        };
        if provider == "codex"
            && let Some(id) = self
                .id_token
                .as_ref()
                .and_then(|id| id.chatgpt_account_id.as_ref())
                .filter(|s| !s.is_empty())
        {
            header["Chatgpt-Account-Id"] = json!(id);
        }
        Some(json!({"authIndex": self.auth_index, "method": "GET", "url": url, "header": header}))
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Reply {
    Files { files: Vec<Entry> },
    Bare(Vec<Entry>),
}

fn entries(body: &str) -> Result<Vec<Entry>, String> {
    match serde_json::from_str::<Reply>(body) {
        Ok(Reply::Files { files } | Reply::Bare(files)) => Ok(files),
        Err(_) => Err("not an auth-files reply".to_owned()),
    }
}

pub fn parse_accounts(body: &str) -> Result<Vec<Account>, String> {
    Ok(entries(body)?.iter().map(Entry::account).collect())
}

#[derive(Deserialize)]
struct ApiReply {
    status_code: u16,
    body: String,
}
#[derive(Deserialize)]
struct ClaudeWindow {
    utilization: f64,
    resets_at: Option<String>,
}
#[derive(Deserialize)]
struct ClaudeUsage {
    five_hour: Option<ClaudeWindow>,
    seven_day: Option<ClaudeWindow>,
}
#[derive(Deserialize)]
struct CodexWindow {
    used_percent: f64,
    reset_at: Option<i64>,
    limit_window_seconds: Option<u64>,
}
#[derive(Deserialize)]
struct CodexLimits {
    primary_window: Option<CodexWindow>,
    secondary_window: Option<CodexWindow>,
}
#[derive(Deserialize)]
struct CodexUsage {
    plan_type: Option<String>,
    rate_limit: Option<CodexLimits>,
}

fn window(pct: f64, resets_at: Option<i64>, now: i64) -> Window {
    Window {
        pct: pct.clamp(0.0, 100.0),
        resets_at,
        reset_in: resets_at.map(|at| {
            let seconds = at.saturating_sub(now).max(0);
            if seconds == 0 {
                "now".to_owned()
            } else {
                let minutes = seconds / 60;
                if minutes >= 1440 {
                    format!("{}d {}h", minutes / 1440, minutes % 1440 / 60)
                } else if minutes >= 60 {
                    format!("{}h {}m", minutes / 60, minutes % 60)
                } else {
                    format!("{}m", minutes.max(1))
                }
            }
        }),
    }
}

fn apply_usage(account: &mut Account, body: &str, now: i64) -> Result<(), &'static str> {
    let document: Value = serde_json::from_str(body).map_err(|_| "malformed usage reply")?;
    let recognized = match account.provider.as_str() {
        "claude" => document.get("five_hour").is_some() || document.get("seven_day").is_some(),
        "codex" => document.get("rate_limit").is_some(),
        _ => false,
    };
    if !recognized {
        return Err("malformed usage reply");
    }
    match account.provider.as_str() {
        "claude" => {
            let usage: ClaudeUsage =
                serde_json::from_value(document).map_err(|_| "malformed usage reply")?;
            let parse = |w: ClaudeWindow| -> Result<Window, &'static str> {
                let reset = w
                    .resets_at
                    .map(|s| {
                        OffsetDateTime::parse(&s, &Rfc3339)
                            .map(|t| t.unix_timestamp())
                            .map_err(|_| "malformed reset time")
                    })
                    .transpose()?;
                Ok(window(w.utilization, reset, now))
            };
            // Commit both together so a malformed window cannot leave partial usage.
            let five = usage.five_hour.map(parse).transpose()?;
            let seven = usage.seven_day.map(parse).transpose()?;
            account.five_hour = five;
            account.seven_day = seven;
        }
        "codex" => {
            let usage: CodexUsage =
                serde_json::from_value(document).map_err(|_| "malformed usage reply")?;
            if let Some(plan) = usage.plan_type.filter(|p| !p.is_empty()) {
                account.plan = Some(plan);
            }
            if let Some(limits) = usage.rate_limit {
                for (w, primary) in [
                    (limits.primary_window, true),
                    (limits.secondary_window, false),
                ] {
                    if let Some(w) = w {
                        let short = w.limit_window_seconds.map_or(primary, |s| s <= 86400);
                        let slot = if short {
                            &mut account.five_hour
                        } else {
                            &mut account.seven_day
                        };
                        let next = window(w.used_percent, w.reset_at, now);
                        if slot.as_ref().is_none_or(|old| next.pct > old.pct) {
                            *slot = Some(next);
                        }
                    }
                }
            }
        }
        _ => unreachable!("only allowlisted providers have usage"),
    }
    Ok(())
}

fn severity(status: &str) -> u8 {
    match status {
        "active" => 0,
        "error" => 4,
        "unavailable" => 3,
        "disabled" => 2,
        _ => 1,
    }
}

/// Aggregate maxima for usage, summed counts, then readings keyed by provider:email.
pub fn readings(accounts: &[Account], updated: u64, stale: bool) -> Vec<Reading> {
    let reading = |account: &str, field: &str, value: String, bar: f64| {
        Reading::new(CLIPROXYAPI, account, field, value, bar, updated, stale)
    };
    let worst = accounts
        .iter()
        .map(|a| a.status.as_str())
        .max_by_key(|status| severity(status))
        .unwrap_or("none");
    let busiest = accounts.iter().map(|a| a.recent_10m).max().unwrap_or(0);
    let sum = |get: fn(&Account) -> u64| {
        accounts
            .iter()
            .map(get)
            .fold(0u64, u64::saturating_add)
            .to_string()
    };
    let mut out = vec![
        reading("", "status", worst.to_owned(), -1.0),
        reading("", "success", sum(|a| a.success), -1.0),
        reading("", "failed", sum(|a| a.failed), -1.0),
        reading("", "recent_10m", sum(|a| a.recent_10m), -1.0),
    ];
    for (field, short) in [("five_hour_pct", true), ("seven_day_pct", false)] {
        if let Some(pct) = accounts
            .iter()
            .filter_map(|a| {
                (if short { &a.five_hour } else { &a.seven_day })
                    .as_ref()
                    .map(|w| w.pct)
            })
            .reduce(f64::max)
        {
            out.push(reading("", field, format!("{pct:.0}%"), pct / 100.0));
        }
    }
    for account in accounts {
        let who = account.key.as_str();
        let bar = if busiest == 0 {
            0.0
        } else {
            account.recent_10m as f64 / busiest as f64
        };
        out.push(reading(who, "status", account.status.clone(), -1.0));
        out.push(reading(who, "success", account.success.to_string(), -1.0));
        out.push(reading(who, "failed", account.failed.to_string(), -1.0));
        out.push(reading(
            who,
            "recent_10m",
            account.recent_10m.to_string(),
            bar,
        ));
        if let Some(plan) = &account.plan {
            out.push(reading(who, "plan", plan.clone(), -1.0));
        }
        for (name, w) in [
            ("five_hour", &account.five_hour),
            ("seven_day", &account.seven_day),
        ] {
            if let Some(w) = w {
                out.push(reading(
                    who,
                    &format!("{name}_pct"),
                    format!("{:.0}%", w.pct),
                    w.pct / 100.0,
                ));
                if let Some(reset) = &w.reset_in {
                    out.push(reading(who, &format!("{name}_reset"), reset.clone(), -1.0));
                }
            }
        }
    }
    out
}

/// Management requests only. Implementations must not log headers or request bodies.
pub trait Fetch: Send + Sync {
    fn request(
        &self,
        _: &super::network::GetRequest<'_>,
    ) -> Result<String, super::network::FetchError> {
        Err(super::network::FetchError::Transport)
    }
    fn get(&self, url: &str, bearer: &str) -> Result<String, String>;
    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<String, String>;
}

pub struct HttpFetch {
    agent: ureq::Agent,
}

impl HttpFetch {
    pub fn new() -> HttpFetch {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .max_redirects(0)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        HttpFetch { agent }
    }
}

impl Default for HttpFetch {
    fn default() -> HttpFetch {
        HttpFetch::new()
    }
}

fn http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        ureq::Error::Timeout(_) => "request timed out (15 s)".to_owned(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => TLS_ERROR.to_owned(),
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => {
            "hub redirect refused".to_owned()
        }
        ureq::Error::HostNotFound => "hub host could not be resolved".to_owned(),
        ureq::Error::Io(ref e) if e.to_string().to_ascii_lowercase().contains("certificate") => {
            TLS_ERROR.to_owned()
        }
        ureq::Error::Io(ref e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            "connection refused".to_owned()
        }
        _ => "hub request failed".to_owned(),
    }
}

/// Even an alternate Fetch must not expose its raw error, which may contain secrets.
fn safe_fetch_error(error: String) -> String {
    if let Some(code) = error
        .strip_prefix("HTTP ")
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|c| (100..600).contains(c))
    {
        return format!("HTTP {code}");
    }
    match error.as_str() {
        TLS_ERROR
        | "request timed out (15 s)"
        | "hub redirect refused"
        | "hub host could not be resolved"
        | "connection refused" => error,
        _ => "hub request failed".to_owned(),
    }
}

fn response_body(mut response: ureq::http::Response<ureq::Body>) -> Result<String, String> {
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.body_mut().read_to_string().map_err(http_error)
}

impl Fetch for HttpFetch {
    fn request(
        &self,
        request: &super::network::GetRequest<'_>,
    ) -> Result<String, super::network::FetchError> {
        super::network::get(request)
    }
    fn get(&self, url: &str, bearer: &str) -> Result<String, String> {
        crate::integrations::secrets::credential_transport(url)?;
        response_body(
            self.agent
                .get(url)
                .header("Authorization", format!("Bearer {bearer}"))
                .header("Accept", "application/json")
                .call()
                .map_err(http_error)?,
        )
    }
    fn post_json(&self, url: &str, bearer: &str, body: &Value) -> Result<String, String> {
        crate::integrations::secrets::credential_transport(url)?;
        response_body(
            self.agent
                .post(url)
                .header("Authorization", format!("Bearer {bearer}"))
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .send(body.to_string())
                .map_err(http_error)?,
        )
    }
}

fn hub_url(base: &str) -> Result<Url, String> {
    Url::parse(base)
        .ok()
        .filter(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host().is_some()
                && u.username().is_empty()
                && u.password().is_none()
                && u.query().is_none()
                && u.fragment().is_none()
        })
        .ok_or_else(|| {
            "use an http:// or https:// hub URL without credentials, query or fragment".to_owned()
        })
}

pub fn insecure_http(base: &str) -> bool {
    hub_url(base).is_ok_and(|u| {
        u.scheme() == "http"
            && !match u.host() {
                Some(Host::Ipv4(ip)) => ip.is_loopback(),
                Some(Host::Ipv6(ip)) => {
                    ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v| v.is_loopback())
                }
                Some(Host::Domain(host)) => {
                    host.eq_ignore_ascii_case("localhost")
                        || host.eq_ignore_ascii_case("localhost.")
                }
                None => false,
            }
    })
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProxyInfo {
    pub url: String,
    pub insecure_http: bool,
    /// The last 4 characters of the management key, never the key.
    pub key_hint: Option<String>,
    /// True after a good auth-files poll, even if an individual provider refused usage.
    pub ok: Option<bool>,
    pub fails: u32,
    pub stale: bool,
    pub error: Option<String>,
    pub polled: Option<u64>,
    pub accounts: Vec<Account>,
}

#[derive(Clone)]
pub struct CliProxy {
    base: String,
    keys: Arc<dyn KeyStore>,
    fetch: Arc<dyn Fetch>,
    last: Vec<Reading>,
    info: ProxyInfo,
    refresh_gate: RefreshGate,
}

impl CliProxy {
    pub fn new(base: &str, keys: Arc<dyn KeyStore>, fetch: Box<dyn Fetch>) -> CliProxy {
        let base = base.trim().trim_end_matches('/').to_owned();
        CliProxy {
            info: ProxyInfo {
                url: base.clone(),
                insecure_http: insecure_http(&base),
                ..ProxyInfo::default()
            },
            base,
            keys,
            fetch: fetch.into(),
            last: Vec::new(),
            refresh_gate: RefreshGate::default(),
        }
    }

    fn fetch_accounts(&mut self, now: i64) -> Result<Vec<Account>, String> {
        hub_url(&self.base)?;
        crate::integrations::secrets::credential_transport(&self.base)?;
        let key = self
            .keys
            .get_for(&self.base)
            .map_err(|_| "could not read the management key from the credential store")?
            .filter(|key| !key.is_empty())
            .ok_or("no management key is set")?;
        self.info.key_hint = Some(key_hint(&key));
        let body = self
            .fetch
            .get(&format!("{}{AUTH_FILES}", self.base), &key)
            .map_err(safe_fetch_error)?;
        let entries = entries(&body)?;
        let mut accounts: Vec<_> = entries.iter().map(Entry::account).collect();
        let url = format!("{}{API_CALL}", self.base);
        // Bounded scoped batches retain auth-files ordering and share the pooled client.
        for (entries, accounts) in entries
            .chunks(CONCURRENCY)
            .zip(accounts.chunks_mut(CONCURRENCY))
        {
            thread::scope(|scope| {
                for (entry, account) in entries.iter().zip(accounts) {
                    account.insecure_http = self.info.insecure_http;
                    if entry.disabled {
                        continue;
                    }
                    if entry.status.eq_ignore_ascii_case("error") {
                        // The hub's status_message can carry raw provider error text, so it is never shown.
                        account.usage_error = Some(
                            "The hub can't sign in to this account. Sign in to it again in CLIProxyAPI."
                                .to_owned(),
                        );
                        continue;
                    }
                    let Some(request) = entry.usage_request(&account.provider) else {
                        continue;
                    };
                    let (fetch, url, key) = (&*self.fetch, &url, &key);
                    scope.spawn(move || {
                        let result = if entry.auth_index.is_empty() {
                            Err("missing auth index".to_owned())
                        } else {
                            fetch
                                .post_json(url, key, &request)
                                .map_err(safe_fetch_error)
                                .and_then(|body| {
                                    let reply: ApiReply = serde_json::from_str(&body)
                                        .map_err(|_| "malformed api-call reply")?;
                                    if matches!(reply.status_code, 401 | 403) {
                                        return Err(format!(
                                            "The service refused this account's sign-in (HTTP {}). Sign in to it again in CLIProxyAPI.",
                                            reply.status_code
                                        ));
                                    }
                                    if !(200..300).contains(&reply.status_code) {
                                        return Err(format!(
                                            "provider refused (HTTP {})",
                                            reply.status_code
                                        ));
                                    }
                                    apply_usage(account, &reply.body, now).map_err(str::to_owned)
                                })
                        };
                        account.usage_error = result.err();
                    });
                }
            });
        }
        Ok(accounts)
    }

    pub fn poll_at(&mut self, now: SystemTime) -> Vec<Reading> {
        let polled = unix_ms(now);
        self.info.polled = Some(polled);
        match self.fetch_accounts((polled / 1000) as i64) {
            Ok(accounts) => {
                self.last = readings(&accounts, polled, false);
                self.info.accounts = accounts;
                self.info.ok = Some(true);
                self.info.fails = 0;
                self.info.stale = false;
                self.info.error = None;
            }
            Err(error) => {
                self.info.ok = Some(false);
                self.info.fails = self.info.fails.saturating_add(1);
                self.info.error = Some(error);
                if self.info.fails >= FAILS_TO_STALE {
                    self.info.stale = true;
                    for reading in &mut self.last {
                        reading.stale = true;
                    }
                }
            }
        }
        self.last.clone()
    }
}

impl Source for CliProxy {
    fn id(&self) -> &'static str {
        CLIPROXYAPI
    }
    fn poll(&mut self) -> Vec<Reading> {
        self.poll_at(SystemTime::now())
    }
    fn refresh(&mut self) -> Vec<Reading> {
        // Also gate execution: requests queued behind a slow poll must not bunch up
        // into provider calls less than 15 s apart when that poll finally finishes.
        if self.refresh_gate.allow(std::time::Instant::now()) {
            self.poll()
        } else {
            self.last.clone()
        }
    }
    fn info(&self) -> Info {
        Info::Proxy(self.info.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_uses_platform_trust_a_fifteen_second_timeout_and_no_redirects() {
        let fetch = HttpFetch::new();
        let config = fetch.agent.config();
        assert!(matches!(
            config.tls_config().root_certs(),
            ureq::tls::RootCerts::PlatformVerifier
        ));
        assert_eq!(config.timeouts().global, Some(Duration::from_secs(15)));
        assert_eq!(config.max_redirects(), 0);
    }

    #[test]
    fn http_errors_are_plain_and_never_include_wrapped_details() {
        assert_eq!(http_error(ureq::Error::Tls("fixture-secret")), TLS_ERROR);
        assert_eq!(
            http_error(ureq::Error::Timeout(ureq::Timeout::Global)),
            "request timed out (15 s)"
        );
        assert_eq!(http_error(ureq::Error::StatusCode(401)), "HTTP 401");
        assert_eq!(
            http_error(ureq::Error::BadUri("fixture-secret".to_owned())),
            "hub request failed"
        );
        assert_eq!(
            http_error(ureq::Error::Io(std::io::Error::other(
                "invalid peer certificate: fixture-secret"
            ))),
            TLS_ERROR
        );
    }
}
