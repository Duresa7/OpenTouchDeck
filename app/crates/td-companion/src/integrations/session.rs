//! The Integrations screen as one window shows it: the list rows with their switches, and one
//! open detail page that sets up, saves, checks and forgets. Ported from the 1.x app's
//! `lib/integrations/model.ts`, `detail.ts` and `switch.ts`, so the rules are tested without a
//! window.
//!
//! The switch writes only `enabled`. Flipping it on before an Integration is set up opens the
//! detail page with a [`Setup`] that enables on the first save that sets it up; flipping it off
//! (or closing the page) cancels that Setup. One Save sends the form's settings and the secrets
//! typed into it together, through `Setup::save`, which never writes `enabled` from the form: a
//! stale form can't undo a switch. "Removed" is turned off with the secret forgotten.
//!
//! Every Integration call is synchronous, so it runs on `spawn_blocking`. Writes from this window
//! run one at a time in the order they were asked for, and the switch is locked while one runs.
use super::{Error, IntegrationSummary, IntegrationView, Service, Setup, SetupCancel};
use crate::{
    companion::{Change, Handle},
    devices::DeviceId,
};
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use td_host::integrations::{CliProxyDetail, ConfigField};
use tokio::sync::{broadcast::error::RecvError, watch};
use zeroize::Zeroizing;

/// Check now asks the hub at most this often, the same floor the Companion keeps.
pub const REFRESH_FLOOR: Duration = Duration::from_secs(15);
/// The custom Integration's sources are the user's own, so it may check every second.
pub const CUSTOM_REFRESH_FLOOR: Duration = Duration::from_secs(1);
pub const FLOOR_TEXT: &str =
    "Checks are at least 15 s apart, because each one queries your AI services.";
pub const CUSTOM_FLOOR_TEXT: &str = "Checks are at least 1 s apart.";
/// The status words while a custom command waits for its approval dialog.
pub const WAITING_FOR_APPROVAL: &str = "Waiting for approval on this PC";
/// The status message the custom Integration reports while it waits.
const APPROVAL_MESSAGE: &str = "Waiting for approval on this PC.";
const RESTART: &str = "Retry, or restart Open TouchDeck.";
/// The switch's field. The form never holds it: the switch writes it.
pub const ENABLED: &str = "enabled";

// ---------------------------------------------------------------------------------------------
// Copy and the schema form

struct FieldCopy {
    label: &'static str,
    placeholder: Option<&'static str>,
    unit: Option<&'static str>,
}

fn field_copy(integration: &str, field: &str) -> Option<FieldCopy> {
    let copy = |label, placeholder, unit| {
        Some(FieldCopy {
            label,
            placeholder,
            unit,
        })
    };
    match (integration, field) {
        ("custom", "sources") => copy("Custom sources (JSON)", Some("[]"), None),
        ("cliproxyapi", "url") => copy("Hub address", Some("https://hub.example.com"), None),
        ("cliproxyapi", "interval_s") => copy("Check every", None, Some("s")),
        ("cliproxyapi", "management_key") => copy("Management key", None, None),
        _ => None,
    }
}

/// The one line under an Integration's name.
pub fn line_of(integration: &str) -> &'static str {
    match integration {
        "volume" => "Shows and sets this PC's volume.",
        "media" => "Shows what's playing on this PC, with play, pause and skip.",
        "this_pc" => "Shows this PC's CPU, memory, GPU, disk and network use.",
        "custom" => "Read values from HTTP or a command approved on this PC.",
        "cliproxyapi" => {
            "Shows how much of each AI account's limit is used, read from your CLIProxyAPI hub."
        }
        _ => "",
    }
}

/// The heading over an Integration's settings form.
pub fn settings_title(integration: &str) -> &'static str {
    match integration {
        "cliproxyapi" => "Connection",
        "custom" => "Sources",
        _ => "Settings",
    }
}

fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn humanize(name: &str) -> String {
    sentence(name.replace('_', " ").trim())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Boolean,
    Integer,
    Number,
    Text,
    Json,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormField {
    pub name: String,
    pub kind: FieldKind,
    pub label: String,
    pub placeholder: Option<&'static str>,
    pub unit: Option<&'static str>,
}

/// The form an Integration's schema builds: its settings, then its write-only secret fields.
/// Never `enabled`.
pub fn form_of(integration: &str, schema: &[ConfigField]) -> (Vec<FormField>, Vec<FormField>) {
    let field = |f: &ConfigField| {
        let copy = field_copy(integration, &f.name);
        FormField {
            name: f.name.clone(),
            kind: match f.kind.as_str() {
                "boolean" => FieldKind::Boolean,
                "integer" => FieldKind::Integer,
                "number" => FieldKind::Number,
                "json" => FieldKind::Json,
                _ => FieldKind::Text,
            },
            label: copy
                .as_ref()
                .map_or_else(|| humanize(&f.name), |c| c.label.into()),
            placeholder: copy.as_ref().and_then(|c| c.placeholder),
            unit: copy.as_ref().and_then(|c| c.unit),
        }
    };
    let settings = schema.iter().filter(|f| f.name != ENABLED);
    let (secrets, fields): (Vec<_>, Vec<_>) = settings.partition(|f| f.secret);
    (
        fields.into_iter().map(field).collect(),
        secrets.into_iter().map(field).collect(),
    )
}

/// Whether an Integration has a settings page: config beyond its switch.
pub fn has_settings(summary: &IntegrationSummary) -> bool {
    summary.config_schema.iter().any(|f| f.name != ENABLED)
}

/// What a form control holds: a switch holds a boolean, every other field the text typed in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Bool(bool),
    Text(String),
}
impl FieldValue {
    fn text(&self) -> &str {
        match self {
            Self::Text(text) => text.trim(),
            Self::Bool(_) => "",
        }
    }
}
pub type FormValues = BTreeMap<String, FieldValue>;

/// The form's starting values, from the config the Companion holds.
pub fn values_of(fields: &[FormField], config: &Value) -> FormValues {
    fields
        .iter()
        .map(|field| {
            let value = config.get(&field.name);
            let shown = match field.kind {
                FieldKind::Boolean => FieldValue::Bool(value == Some(&Value::Bool(true))),
                FieldKind::Json => FieldValue::Text(
                    serde_json::to_string_pretty(value.unwrap_or(&Value::Array(Vec::new())))
                        .unwrap_or_default(),
                ),
                _ => FieldValue::Text(match value {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Number(number)) => number.to_string(),
                    _ => String::new(),
                }),
            };
            (field.name.clone(), shown)
        })
        .collect()
}

fn whole(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Each field the schema's kinds refuse, with the sentence to show under it.
pub fn validate(fields: &[FormField], values: &FormValues) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    for field in fields {
        let text = values.get(&field.name).map_or("", FieldValue::text);
        let problem = match field.kind {
            FieldKind::Json if serde_json::from_str::<Value>(text).is_err() => {
                Some("Enter valid JSON.")
            }
            FieldKind::Integer if whole(text).is_none() => Some("Enter a whole number."),
            FieldKind::Number if number(text).is_none() => Some("Enter a number."),
            _ => None,
        };
        if let Some(problem) = problem {
            errors.insert(field.name.clone(), problem.into());
        }
    }
    errors
}

/// The config to send: only the schema's settings, never a secret or `enabled`. Validate first.
pub fn serialize(fields: &[FormField], values: &FormValues) -> Value {
    let mut config = serde_json::Map::new();
    for field in fields {
        let value = values.get(&field.name);
        let text = value.map_or("", FieldValue::text);
        let json = match field.kind {
            FieldKind::Boolean => Value::Bool(value == Some(&FieldValue::Bool(true))),
            FieldKind::Json => serde_json::from_str(text).unwrap_or(Value::Null),
            FieldKind::Integer => whole(text).map_or(Value::Null, Value::from),
            FieldKind::Number => number(text)
                .and_then(serde_json::Number::from_f64)
                .map_or(Value::Null, Value::Number),
            FieldKind::Text => Value::String(match value {
                Some(FieldValue::Text(text)) => text.clone(),
                _ => String::new(),
            }),
        };
        config.insert(field.name.clone(), json);
    }
    Value::Object(config)
}

/// How a key would travel to an address, as the Companion decides it: https always, plain http
/// only to this PC or a private network address typed as an IP, and never to anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Secure,
    Local,
    Unencrypted,
    Refused,
    Invalid,
}

pub fn transport_of(address: &str) -> Transport {
    let Ok(url) = url::Url::parse(address.trim()) else {
        return Transport::Invalid;
    };
    match url.scheme() {
        "https" => return Transport::Secure,
        "http" => {}
        _ => return Transport::Invalid,
    }
    match url.host() {
        Some(url::Host::Domain("localhost")) => Transport::Local,
        Some(url::Host::Ipv4(ip)) if ip.is_loopback() => Transport::Local,
        Some(url::Host::Ipv6(ip)) if ip.is_loopback() => Transport::Local,
        Some(url::Host::Ipv4(ip)) if ip.is_private() => Transport::Unencrypted,
        Some(url::Host::Ipv6(ip)) if (ip.segments()[0] & 0xfe00) == 0xfc00 => {
            Transport::Unencrypted
        }
        _ => Transport::Refused,
    }
}

/// The warning for an address a key travels to, or `None` when it travels safely.
pub fn transport_warning(address: &str) -> Option<&'static str> {
    match transport_of(address) {
        Transport::Unencrypted => Some(
            "This address isn't on this PC and uses http://, so the key is sent unencrypted. Use https://.",
        ),
        Transport::Refused => Some(
            "This address uses http:// and isn't on your network, so the key isn't sent. Use https://.",
        ),
        Transport::Secure | Transport::Local | Transport::Invalid => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Status and the switch

/// A status box's colour: green works, red doesn't, amber is on its way or waits for you.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Bad,
    Wait,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub words: String,
    /// The status box to show, or `None` when the switch says it all: the Integration is off.
    pub tone: Option<Tone>,
    /// Connected, so its values are live.
    pub live: bool,
    /// No credentials or sources yet.
    pub needs_setup: bool,
    /// The Companion's reason, for Details.
    pub detail: Option<String>,
}

/// The status to show. `pending` is a switch turned on before setup: it reads as on and waiting
/// for setup.
pub fn status_of(summary: &IntegrationSummary, pending: bool) -> Status {
    let state = summary.status.state.as_str();
    let status = |words: &str, tone, live, needs_setup, detail| Status {
        words: words.into(),
        tone,
        live,
        needs_setup,
        detail,
    };
    if pending && !summary.enabled {
        return status(
            "Finish setup",
            Some(Tone::Wait),
            false,
            state == "not_configured",
            None,
        );
    }
    if state == "not_configured" {
        return if summary.enabled {
            status("Finish setup", Some(Tone::Wait), false, true, None)
        } else {
            status("Not set up", None, false, true, None)
        };
    }
    if !summary.enabled {
        return status("Off", None, false, false, None);
    }
    match state {
        "ok" => status("Connected", Some(Tone::Ok), true, false, None),
        "error" if summary.status.message.as_deref() == Some(APPROVAL_MESSAGE) => {
            status(WAITING_FOR_APPROVAL, Some(Tone::Wait), false, false, None)
        }
        "error" => status(
            "Can't connect",
            Some(Tone::Bad),
            false,
            false,
            summary.status.message.clone(),
        ),
        _ => status(
            "Checking…",
            Some(Tone::Wait),
            false,
            false,
            summary.status.message.clone(),
        ),
    }
}

/// What flipping an Integration's switch does: turn it on or off, or open its setup first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flip {
    Enable,
    Disable,
    SetUp,
}

pub fn flip_of(summary: &IntegrationSummary, on: bool) -> Flip {
    if !on {
        Flip::Disable
    } else if summary.status.state == "not_configured" {
        Flip::SetUp
    } else {
        Flip::Enable
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchView {
    pub checked: bool,
    /// A write is on its way, or an enable waits for setup to finish.
    pub busy: bool,
    /// A write is on its way or queued, so the switch can't be flipped. An enable waiting for
    /// setup can still be undone.
    pub locked: bool,
}

/// What the switch shows: the saved state, the state it's saving, or on while an enable waits
/// for setup.
pub fn switch_view(
    enabled: bool,
    saving: Option<bool>,
    pending: bool,
    writes: usize,
) -> SwitchView {
    let locked = saving.is_some() || writes > 0;
    match saving {
        Some(on) => SwitchView {
            checked: on,
            busy: true,
            locked,
        },
        None if pending && !enabled => SwitchView {
            checked: true,
            busy: true,
            locked,
        },
        None => SwitchView {
            checked: enabled,
            busy: locked,
            locked,
        },
    }
}

/// "3 accounts", "1 account".
fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

/// How long ago, as "12 s ago", "5 min ago" or "2 hr ago".
pub fn ago(then_ms: u64, now_ms: u64) -> String {
    let seconds = (now_ms.saturating_sub(then_ms) as f64 / 1000.0).round() as u64;
    if seconds < 90 {
        return format!("{seconds} s ago");
    }
    let minutes = (seconds as f64 / 60.0).round() as u64;
    if minutes < 90 {
        format!("{minutes} min ago")
    } else {
        format!("{} hr ago", (minutes as f64 / 60.0).round() as u64)
    }
}

// ---------------------------------------------------------------------------------------------
// Views

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub icon: String,
    /// The line under the name: what it does, or, while it's on, what's wrong with it.
    pub line: String,
    pub status: Status,
    pub switch: SwitchView,
    /// It has a settings page.
    pub settings: bool,
    /// Devices this Integration's values couldn't reach.
    pub delivery_failures: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Ok,
    Info,
    Error,
}

/// The part of the page an outcome belongs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Settings,
    Secret,
    Status,
    Page,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    /// The error behind it, for Details.
    pub detail: Option<String>,
    /// Names the outcome for tests.
    pub event: &'static str,
}
impl Notice {
    fn new(kind: NoticeKind, text: impl Into<String>, event: &'static str) -> Self {
        Self {
            kind,
            text: text.into(),
            detail: None,
            event,
        }
    }
    fn with(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    /// Which part of the page it shows under.
    pub fn at(&self) -> Outcome {
        match self.event {
            "saving" | "saved" | "save-failed" | "secret-forgotten" => Outcome::Settings,
            "forget-failed" => Outcome::Secret,
            event if event.starts_with("secret") => Outcome::Secret,
            event if event.starts_with("refresh") => Outcome::Status,
            _ => Outcome::Page,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Load {
    #[default]
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Save,
    Secret,
    Refresh,
}

/// A write-only secret: whether one is stored and whether a new value is typed. Never its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretView {
    pub field: FormField,
    /// Stored, so it shows "Saved" with Replace and Forget.
    pub set: bool,
    /// A value is typed and goes with the next Save.
    pub entered: bool,
}

/// A Device that didn't take this Integration's last values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryProblem {
    pub device: String,
    pub text: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshView {
    /// Check now may ask: the Integration is on and set up.
    pub usable: bool,
    pub line: String,
    /// When Check now may ask again.
    pub after: Option<Instant>,
}
impl RefreshView {
    pub fn waiting(&self, now: Instant) -> bool {
        self.after.is_some_and(|after| now < after)
    }
}

/// The command a custom source waits to run, as its approval dialog shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub id: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, PartialEq)]
pub struct DetailView {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub line: &'static str,
    pub settings_title: &'static str,
    /// Opened to set it up, so focus goes to the first empty field.
    pub set_up: bool,
    pub load: Load,
    pub fields: Vec<FormField>,
    pub secrets: Vec<SecretView>,
    pub values: FormValues,
    /// The values as the Companion holds them.
    pub saved: FormValues,
    pub errors: BTreeMap<String, String>,
    pub busy: Option<Busy>,
    pub notice: Option<Notice>,
    pub status: Status,
    pub switch: SwitchView,
    pub deliveries: Vec<DeliveryProblem>,
    /// The custom command waiting for its approval dialog on this PC.
    pub approval: Option<Approval>,
    /// Check now, for an Integration that offers it.
    pub refresh: Option<RefreshView>,
    pub usage: Option<CliProxyDetail>,
}
impl std::fmt::Debug for DetailView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetailView")
            .field("id", &self.id)
            .field("load", &self.load)
            .field("fields", &self.fields)
            .field("secrets", &self.secrets)
            .field("values", &self.values)
            .field("errors", &self.errors)
            .field("busy", &self.busy)
            .field("notice", &self.notice)
            .field("status", &self.status)
            .field("switch", &self.switch)
            .field("deliveries", &self.deliveries)
            .field("approval", &self.approval)
            .field("refresh", &self.refresh)
            .finish_non_exhaustive()
    }
}
impl DetailView {
    /// The form differs from what the Companion holds, or a secret is typed.
    pub fn dirty(&self) -> bool {
        self.fields
            .iter()
            .any(|f| self.values.get(&f.name) != self.saved.get(&f.name))
            || self.secrets.iter().any(|s| s.entered)
    }
    /// Save may run: loaded, nothing running, the switch not writing and something to save.
    pub fn can_save(&self) -> bool {
        self.load == Load::Ready && self.busy.is_none() && !self.switch.locked && self.dirty()
    }
    /// The words beside the status box. The box already says connected.
    pub fn header_line(&self, now_ms: u64) -> String {
        if self.status.tone != Some(Tone::Ok) {
            return self.status.words.clone();
        }
        match &self.usage {
            Some(usage) => format!(
                "{} · checked {}",
                plural(usage.accounts.len(), "account"),
                ago(usage.polled_ms, now_ms)
            ),
            None => "Running".into(),
        }
    }
    /// The transport warning under a field, for an Integration with a key.
    pub fn warning(&self, field: &str) -> Option<&'static str> {
        let field = self.fields.iter().find(|f| f.name == field)?;
        if self.secrets.is_empty() || field.kind != FieldKind::Text {
            return None;
        }
        transport_warning(self.values.get(&field.name)?.text())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntegrationsView {
    pub rows: Vec<Row>,
    pub detail: Option<DetailView>,
    /// What the last switch flip said, when it failed.
    pub notice: Option<Notice>,
}
impl IntegrationsView {
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.id == id)
    }
}

// ---------------------------------------------------------------------------------------------
// The session

struct Detail {
    id: String,
    /// Bumped whenever the page is replaced, so a late answer for an older page is dropped.
    opened: u64,
    set_up: bool,
    /// Holds enable intent while the switch waits for setup. Taken while a save runs.
    setup: Option<Setup>,
    /// Cancels that intent, even while a save holds the Setup. Dropping the page cancels it.
    cancel: Option<SetupCancel>,
    view: Option<IntegrationView>,
    load: Load,
    fields: Vec<FormField>,
    secret_fields: Vec<FormField>,
    values: FormValues,
    saved: FormValues,
    errors: BTreeMap<String, String>,
    /// Typed secrets, sent with the next Save and never shown.
    entered: BTreeMap<String, Zeroizing<String>>,
    busy: Option<Busy>,
    notice: Option<Notice>,
    refresh_after: Option<Instant>,
    reloading: bool,
    stale: bool,
}
impl Detail {
    fn dirty(&self) -> bool {
        self.fields
            .iter()
            .any(|f| self.values.get(&f.name) != self.saved.get(&f.name))
    }
    /// Takes a fresh view. `keep` holds on to what's typed in a changed form.
    fn take(&mut self, view: IntegrationView, keep: bool) {
        let (fields, secret_fields) = form_of(&self.id, &view.summary.config_schema);
        let saved = values_of(&fields, &view.config);
        let keep = keep && self.view.is_some() && self.dirty();
        if keep {
            for (name, value) in &saved {
                self.values
                    .entry(name.clone())
                    .or_insert_with(|| value.clone());
            }
            self.values.retain(|name, _| saved.contains_key(name));
        } else {
            self.values = saved.clone();
            self.errors.clear();
        }
        self.fields = fields;
        self.secret_fields = secret_fields;
        self.saved = saved;
        self.load = Load::Ready;
        self.view = Some(view);
        self.draft_secrets();
    }
    /// The custom Integration's secret fields come from the header references in its sources as
    /// typed, so a secret can go in the same Save as the source that names it. Sources that
    /// aren't valid JSON yet keep the fields shown before.
    fn draft_secrets(&mut self) {
        if self.id == "custom"
            && let Some(FieldValue::Text(sources)) = self.values.get("sources")
            && let Ok(Value::Array(sources)) = serde_json::from_str::<Value>(sources)
        {
            let names: std::collections::BTreeSet<&str> = sources
                .iter()
                .filter(|source| source["input"]["type"] == "http")
                .filter_map(|source| source["input"]["headers"].as_object())
                .flat_map(|headers| headers.values().filter_map(Value::as_str))
                .collect();
            let schema: Vec<ConfigField> = names
                .into_iter()
                .map(|name| ConfigField {
                    name: name.into(),
                    kind: "string".into(),
                    secret: true,
                })
                .collect();
            self.secret_fields = form_of(&self.id, &schema).1;
        }
        let fields = &self.secret_fields;
        self.entered
            .retain(|name, _| fields.iter().any(|f| &f.name == name));
    }
    /// Keeps a Setup's enable intent with this page.
    fn intend(&mut self, setup: Setup) {
        self.cancel = Some(setup.canceller());
        self.setup = Some(setup);
    }
    /// Cancels the enable intent, including a save of it that's on its way.
    fn drop_intent(&mut self) {
        self.setup = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }
    fn secret_set(&self, field: &str) -> bool {
        self.view
            .as_ref()
            .and_then(|v| v.secrets.get(field))
            .is_some_and(|s| s.set)
    }
    fn label(&self, field: &str) -> String {
        self.secret_fields
            .iter()
            .chain(&self.fields)
            .find(|f| f.name == field)
            .map_or_else(|| field.to_owned(), |f| f.label.to_lowercase())
    }
}

impl Drop for Detail {
    fn drop(&mut self) {
        self.drop_intent();
    }
}

#[derive(Default)]
struct State {
    summaries: Vec<IntegrationSummary>,
    /// The state each switch is writing.
    saving: BTreeMap<String, bool>,
    /// Writes queued or running, by Integration.
    writes: BTreeMap<String, usize>,
    /// The Integration whose switch was turned on before it was set up.
    pending: Option<String>,
    notice: Option<Notice>,
    detail: Option<Detail>,
    opened: u64,
}
impl State {
    fn summary(&self, id: &str) -> Option<&IntegrationSummary> {
        self.summaries.iter().find(|s| s.id == id)
    }
    fn switch(&self, summary: &IntegrationSummary) -> SwitchView {
        switch_view(
            summary.enabled,
            self.saving.get(&summary.id).copied(),
            self.pending.as_deref() == Some(summary.id.as_str()),
            self.writes.get(&summary.id).copied().unwrap_or(0),
        )
    }
    fn count(&mut self, id: &str, by: isize) {
        let next = self.writes.get(id).copied().unwrap_or(0) as isize + by;
        if next > 0 {
            self.writes.insert(id.into(), next as usize);
        } else {
            self.writes.remove(id);
        }
    }
    fn detail_of(&mut self, id: &str, opened: u64) -> Option<&mut Detail> {
        self.detail
            .as_mut()
            .filter(|d| d.id == id && d.opened == opened)
    }
}

struct Inner {
    handle: Handle,
    state: Mutex<State>,
    view: watch::Sender<IntegrationsView>,
    /// This window's writes run one at a time, in the order they were asked for.
    writes: tokio::sync::Mutex<()>,
}

/// The Integrations screen of one window. Clones share their state.
#[derive(Clone)]
pub struct IntegrationsSession {
    inner: Arc<Inner>,
}

/// A short reason for Details. The Integration module's errors carry no text of their own.
fn describe(error: &Error) -> String {
    match error {
        Error::NotFound => "That Integration isn't available.".into(),
        Error::ConfigInvalid => "The settings weren't accepted.".into(),
        Error::ConfigField { message, .. } => (*message).into(),
        Error::Credentials => "Couldn't read this PC's credential store.".into(),
        Error::Storage => "Couldn't save to this PC.".into(),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or(Err(Error::Storage))
}

impl IntegrationsSession {
    /// Starts the window's session from the Companion's cached summaries. Call it inside the Tokio
    /// runtime: it follows the Companion's changes there until the last clone is dropped.
    pub fn new(handle: Handle) -> Self {
        let changes = handle.observe();
        let summaries = handle.integrations().summaries();
        let session = Self {
            inner: Arc::new(Inner {
                handle,
                state: Mutex::new(State {
                    summaries,
                    ..State::default()
                }),
                view: watch::channel(IntegrationsView::default()).0,
                writes: tokio::sync::Mutex::new(()),
            }),
        };
        session.publish();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let weak = Arc::downgrade(&session.inner);
            let closed = session.inner.view.subscribe();
            runtime.spawn(follow(weak, changes, closed));
        }
        session
    }
    pub fn view(&self) -> IntegrationsView {
        self.inner.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<IntegrationsView> {
        self.inner.view.subscribe()
    }
    fn service(&self) -> Service {
        self.inner.handle.integrations().clone()
    }

    /// Changes the state and sends the view it makes, under one lock: a later change can't send
    /// its view before an earlier one's.
    fn change<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self.inner.state.lock();
        let out = change(&mut state);
        self.send(&state);
        out
    }

    fn publish(&self) {
        self.send(&self.inner.state.lock());
    }

    fn send(&self, state: &State) {
        let view = self.render(state);
        #[cfg(any(test, feature = "test-hooks"))]
        let _ = self
            .inner
            .handle
            .checkpoints
            .pass(crate::checkpoints::Checkpoint::IntegrationsPublish);
        self.inner.view.send_if_modified(|current| {
            let changed = *current != view;
            *current = view;
            changed
        });
    }

    fn render(&self, state: &State) -> IntegrationsView {
        let rows = state
            .summaries
            .iter()
            .map(|summary| {
                let status = status_of(summary, false);
                let problem =
                    summary.enabled && matches!(status.tone, Some(Tone::Bad | Tone::Wait));
                Row {
                    id: summary.id.clone(),
                    name: summary.name.clone(),
                    icon: summary.icon.clone(),
                    line: if problem {
                        status.words.clone()
                    } else {
                        line_of(&summary.id).into()
                    },
                    status,
                    switch: state.switch(summary),
                    settings: has_settings(summary),
                    delivery_failures: summary
                        .status
                        .delivery
                        .iter()
                        .filter(|d| d.error.is_some())
                        .count(),
                }
            })
            .collect();
        let detail = state.detail.as_ref().and_then(|detail| {
            let summary = state.summary(&detail.id)?;
            let pending = state.pending.as_deref() == Some(detail.id.as_str());
            Some(self.render_detail(detail, summary, pending, state.switch(summary)))
        });
        IntegrationsView {
            rows,
            detail,
            notice: state.notice.clone(),
        }
    }

    fn render_detail(
        &self,
        detail: &Detail,
        summary: &IntegrationSummary,
        pending: bool,
        switch: SwitchView,
    ) -> DetailView {
        let status = status_of(summary, pending);
        let deliveries = summary
            .status
            .delivery
            .iter()
            .filter_map(|delivery| {
                let error = delivery.error.as_ref()?;
                let name = DeviceId::parse(&delivery.device)
                    .and_then(|id| self.inner.handle.devices().get(&id))
                    .and_then(|device| device.name)
                    .unwrap_or_else(|| delivery.device.clone());
                Some(DeliveryProblem {
                    device: delivery.device.clone(),
                    text: format!("Couldn't send values to {name}. See Details."),
                    detail: error.clone(),
                })
            })
            .collect();
        let approval = (status.words == WAITING_FOR_APPROVAL)
            .then(|| {
                self.inner
                    .handle
                    .actions()
                    .pending()
                    .into_iter()
                    .find(|request| request.device.as_str() == "custom" && request.tile == "custom")
                    .map(|request| Approval {
                        id: request.id,
                        program: request.spec.program,
                        args: request.spec.args,
                    })
            })
            .flatten();
        let refresh = summary.actions.iter().any(|a| a == "refresh").then(|| {
            let usable = summary.enabled && summary.status.state != "not_configured";
            RefreshView {
                usable,
                line: if usable {
                    floor_text(&summary.id).into()
                } else if summary.enabled || pending {
                    "Finish setup to check.".into()
                } else {
                    format!("Turn on {} to check.", summary.name)
                },
                after: detail.refresh_after,
            }
        });
        DetailView {
            id: detail.id.clone(),
            name: summary.name.clone(),
            icon: summary.icon.clone(),
            line: line_of(&detail.id),
            settings_title: settings_title(&detail.id),
            set_up: detail.set_up,
            load: detail.load,
            fields: detail.fields.clone(),
            secrets: detail
                .secret_fields
                .iter()
                .map(|field| SecretView {
                    field: field.clone(),
                    set: detail.secret_set(&field.name),
                    entered: detail.entered.contains_key(&field.name),
                })
                .collect(),
            values: detail.values.clone(),
            saved: detail.saved.clone(),
            errors: detail.errors.clone(),
            busy: detail.busy,
            notice: detail.notice.clone(),
            status,
            switch,
            deliveries,
            approval,
            refresh,
            usage: detail.view.as_ref().and_then(|v| v.detail.clone()),
        }
    }

    /// Follows one Companion change. The session does this itself, so a window only calls it
    /// when it feeds changes by hand. Call it inside the Tokio runtime: the page's reload after a
    /// status change is spawned there.
    pub fn observe(&self, change: &Change) {
        match change {
            Change::IntegrationStatus(event) => {
                let reload = self.change(|state| {
                    state.summaries = self.inner.handle.integrations().summaries();
                    let detail = state.detail.as_mut().filter(|d| d.id == event.0.id)?;
                    if detail.reloading {
                        detail.stale = true;
                        return None;
                    }
                    detail.reloading = true;
                    Some((detail.id.clone(), detail.opened))
                });
                if let Some((id, opened)) = reload
                    && tokio::runtime::Handle::try_current().is_ok()
                {
                    let session = self.clone();
                    tokio::spawn(async move { session.reload(id, opened).await });
                }
            }
            Change::ApprovalRequested(_)
            | Change::ApprovalResolved(_)
            | Change::DeviceAdded(_)
            | Change::DeviceUpdated(_)
            | Change::DeviceRemoved(_) => self.publish(),
            _ => {}
        }
    }

    /// Reads the cached summaries again, after missed changes.
    pub fn sync(&self) {
        self.change(|state| state.summaries = self.inner.handle.integrations().summaries());
    }

    /// Reads the open page's view again, keeping what's typed.
    async fn reload(&self, id: String, opened: u64) {
        loop {
            let _turn = self.inner.writes.lock().await;
            let service = self.service();
            let wanted = id.clone();
            let result = blocking(move || service.view(&wanted)).await;
            let again = self.change(|state| {
                let Some(detail) = state.detail_of(&id, opened) else {
                    return false;
                };
                if let Ok(view) = result {
                    detail.take(view, true);
                }
                let again = std::mem::take(&mut detail.stale);
                detail.reloading = again;
                again
            });
            if !again {
                return;
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // The switch

    /// Flips an Integration's switch. On before setup opens its setup page and turns it on once a
    /// save sets it up; off cancels that, and always turns it off. Only `enabled` is written.
    /// `false` when the switch was locked or the write failed.
    pub async fn flip(&self, id: &str, on: bool) -> bool {
        let (summary, switch, pending) = {
            let state = self.inner.state.lock();
            let Some(summary) = state.summary(id).cloned() else {
                return false;
            };
            let switch = state.switch(&summary);
            let pending = state.pending.as_deref() == Some(id);
            (summary, switch, pending)
        };
        if switch.locked {
            return false;
        }
        if pending && !on {
            self.change(|state| {
                state.pending = None;
                if let Some(detail) = state.detail.as_mut().filter(|d| d.id == id) {
                    detail.drop_intent();
                }
            });
            if !summary.enabled {
                return true;
            }
        }
        if flip_of(&summary, on) == Flip::SetUp {
            if summary.enabled {
                return true;
            }
            self.open_with(id, true).await;
            return true;
        }
        self.change(|state| {
            state.saving.insert(id.into(), on);
            state.count(id, 1);
            state.notice = None;
        });
        // The gate stays held until the result is published, so writes publish in order.
        let _turn = self.inner.writes.lock().await;
        let service = self.service();
        let wanted = id.to_owned();
        let result = blocking(move || service.set_enabled(&wanted, on)).await;
        self.change(|state| {
            state.saving.remove(id);
            state.count(id, -1);
            state.summaries = self.inner.handle.integrations().summaries();
            match &result {
                Ok(view) => {
                    if let Some(detail) = state.detail.as_mut().filter(|d| d.id == id) {
                        detail.take(view.clone(), true);
                    }
                }
                Err(error) => {
                    let text = format!(
                        "Couldn't turn {} {}.",
                        if on { "on" } else { "off" },
                        summary.name
                    );
                    state.notice = Some(
                        Notice::new(NoticeKind::Error, text, "switch-failed").with(describe(error)),
                    );
                }
            }
        });
        result.is_ok()
    }

    // -----------------------------------------------------------------------------------------
    // The detail page

    /// Opens an Integration's settings page, replacing any other. An enable waiting for another
    /// Integration's setup is cancelled.
    pub async fn open(&self, id: &str) -> bool {
        self.open_with(id, false).await
    }

    async fn open_with(&self, id: &str, set_up: bool) -> bool {
        let opened = self.change(|state| {
            state.summary(id)?;
            state.opened += 1;
            if state.pending.as_deref() != Some(id) || set_up {
                state.pending = set_up.then(|| id.to_owned());
            }
            // Opening the same page again keeps an enable still waiting for its setup.
            let carried = state
                .detail
                .take()
                .filter(|d| d.id == id && state.pending.as_deref() == Some(id))
                .and_then(|mut d| {
                    // Moved, not cancelled: the old page's drop must not cancel it.
                    d.cancel = None;
                    d.setup.take()
                });
            state.detail = Some(Detail {
                id: id.into(),
                opened: state.opened,
                set_up,
                cancel: carried.as_ref().map(Setup::canceller),
                setup: carried,
                view: None,
                load: Load::Loading,
                fields: Vec::new(),
                secret_fields: Vec::new(),
                values: FormValues::new(),
                saved: FormValues::new(),
                errors: BTreeMap::new(),
                entered: BTreeMap::new(),
                busy: None,
                notice: None,
                refresh_after: None,
                reloading: false,
                stale: false,
            });
            Some(state.opened)
        });
        let Some(opened) = opened else {
            return false;
        };
        let _turn = self.inner.writes.lock().await;
        let service = self.service();
        let wanted = id.to_owned();
        let result = blocking(move || {
            let setup = set_up.then(|| service.setup(&wanted, true)).transpose()?;
            Ok((service.view(&wanted)?, setup))
        })
        .await;
        self.change(|state| {
            let still_pending = state.pending.as_deref() == Some(id);
            let Some(detail) = state.detail_of(id, opened) else {
                return false;
            };
            match result {
                Ok((view, setup)) => {
                    detail.take(view, true);
                    // A switch turned off while this page opened keeps the Setup from enabling.
                    match setup {
                        Some(setup) if still_pending => detail.intend(setup),
                        Some(setup) => setup.cancel(),
                        None => {}
                    }
                    true
                }
                Err(error) => {
                    detail.load = Load::Failed;
                    detail.notice = Some(
                        Notice::new(
                            NoticeKind::Error,
                            format!("Couldn't load this Integration's settings. {RESTART}"),
                            "load-failed",
                        )
                        .with(describe(&error)),
                    );
                    if set_up {
                        state.pending = None;
                    }
                    false
                }
            }
        })
    }

    /// Closes the settings page. An enable still waiting for its setup is cancelled.
    pub fn close(&self) {
        self.change(|state| {
            if let Some(detail) = state.detail.take()
                && state.pending.as_deref() == Some(detail.id.as_str())
            {
                state.pending = None;
            }
        });
    }

    /// Changes a form field. Its error and the last outcome go.
    pub fn set_value(&self, name: &str, value: FieldValue) {
        self.change(|state| {
            if let Some(detail) = &mut state.detail
                && detail.fields.iter().any(|f| f.name == name)
            {
                detail.values.insert(name.into(), value);
                detail.errors.remove(name);
                detail.notice = None;
                detail.draft_secrets();
            }
        });
    }

    /// Types a secret's new value; it goes with the next Save. An empty value clears what's
    /// typed. The value stays in this session and never reaches the view.
    pub fn enter_secret(&self, field: &str, value: &str) {
        self.change(|state| {
            if let Some(detail) = &mut state.detail
                && detail.secret_fields.iter().any(|f| f.name == field)
            {
                if value.trim().is_empty() {
                    detail.entered.remove(field);
                } else {
                    detail
                        .entered
                        .insert(field.into(), Zeroizing::new(value.to_owned()));
                }
                detail.errors.remove(field);
                detail.notice = None;
            }
        });
    }

    /// Saves the form's settings and its typed secrets together. With a switch waiting for setup,
    /// the save that sets it up also turns it on. `enabled` is never sent from the form, so a
    /// stale form can't undo a switch.
    pub async fn save(&self) -> bool {
        let prepared = self.change(|state| {
            let detail = state.detail.as_mut()?;
            if detail.busy.is_some() || detail.load != Load::Ready {
                return None;
            }
            let errors = validate(&detail.fields, &detail.values);
            if !errors.is_empty() {
                detail.errors = errors;
                detail.notice = None;
                return None;
            }
            let config = serialize(&detail.fields, &detail.values);
            let secrets: BTreeMap<String, String> = detail
                .entered
                .iter()
                .map(|(field, value)| (field.clone(), value.as_str().to_owned()))
                .collect();
            let before: Vec<String> = detail
                .secret_fields
                .iter()
                .filter(|f| detail.secret_set(&f.name) && !secrets.contains_key(&f.name))
                .map(|f| f.name.clone())
                .collect();
            detail.busy = Some(Busy::Save);
            detail.notice = Some(Notice::new(NoticeKind::Info, "Saving…", "saving"));
            let setup = detail.setup.take();
            let (id, opened) = (detail.id.clone(), detail.opened);
            state.count(&id, 1);
            Some((id, opened, config, secrets, before, setup))
        });
        let Some((id, opened, config, secrets, before, setup)) = prepared else {
            return false;
        };
        let intent = setup.is_some();
        // The gate stays held until the result is published, so writes publish in order.
        let _turn = self.inner.writes.lock().await;
        let (setup, result) = {
            let service = self.service();
            let wanted = id.clone();
            tokio::task::spawn_blocking(move || {
                let mut setup = match setup {
                    Some(setup) => setup,
                    None => match service.setup(&wanted, false) {
                        Ok(setup) => setup,
                        Err(error) => return (None, Err((error, None))),
                    },
                };
                let result = setup.save(config, secrets);
                let reread = result
                    .is_err()
                    .then(|| service.view(&wanted).ok())
                    .flatten();
                (Some(setup), result.map_err(|e| (e, reread)))
            })
            .await
            .unwrap_or((None, Err((Error::Storage, None))))
        };
        self.change(|state| {
            state.count(&id, -1);
            state.summaries = self.inner.handle.integrations().summaries();
            let pending = state.pending.as_deref() == Some(id.as_str());
            let Some(detail) = state.detail_of(&id, opened) else {
                return false;
            };
            detail.busy = None;
            match result {
                Ok(view) => {
                    let enabled = view.summary.enabled;
                    detail.take(view, false);
                    detail.entered.clear();
                    let forgotten: Vec<String> = before
                        .iter()
                        .filter(|f| !detail.secret_set(f))
                        .map(|f| detail.label(f))
                        .collect();
                    detail.notice = Some(if forgotten.is_empty() {
                        Notice::new(NoticeKind::Ok, "Saved", "saved")
                    } else {
                        Notice::new(
                            NoticeKind::Info,
                            format!(
                                "Saved. The address changed, so the {} was forgotten. Enter it again.",
                                forgotten.join(" and ")
                            ),
                            "secret-forgotten",
                        )
                    });
                    if pending && enabled {
                        state.pending = None;
                    } else if pending && intent {
                        // Still waiting for setup: keep the enable intent for the next save.
                        state.detail_of(&id, opened).unwrap().setup = setup;
                    }
                    true
                }
                Err((error, reread)) => {
                    if let Some(view) = reread {
                        detail.take(view, true);
                    }
                    if pending && intent {
                        detail.setup = setup;
                    }
                    let check = "Couldn't save. Check the fields, then retry.";
                    detail.notice = Some(match &error {
                        Error::ConfigField { field, message } => {
                            detail.errors.insert((*field).into(), (*message).into());
                            Notice::new(NoticeKind::Error, check, "save-failed").with(message.to_string())
                        }
                        Error::ConfigInvalid => {
                            // Setting up needs every secret: name the missing one.
                            let missing: Vec<String> = detail
                                .secret_fields
                                .iter()
                                .filter(|f| {
                                    !detail.secret_set(&f.name) && !detail.entered.contains_key(&f.name)
                                })
                                .map(|f| f.name.clone())
                                .collect();
                            if intent {
                                for field in missing {
                                    let label = detail.label(&field);
                                    detail.errors.insert(field, format!("Enter the {label}."));
                                }
                            }
                            Notice::new(NoticeKind::Error, check, "save-failed")
                                .with(describe(&error))
                        }
                        error => Notice::new(
                            NoticeKind::Error,
                            format!("Couldn't save. {RESTART}"),
                            "save-failed",
                        )
                        .with(describe(error)),
                    });
                    false
                }
            }
        })
    }

    /// Forgets a stored secret. The Integration stops until one is saved again; with its switch
    /// off too, it counts as removed.
    pub async fn forget_secret(&self, field: &str) -> bool {
        let prepared = self.change(|state| {
            let detail = state.detail.as_mut()?;
            if detail.busy.is_some() || !detail.secret_fields.iter().any(|f| f.name == field) {
                return None;
            }
            detail.busy = Some(Busy::Secret);
            detail.notice = None;
            let (id, opened) = (detail.id.clone(), detail.opened);
            state.count(&id, 1);
            Some((id, opened))
        });
        let Some((id, opened)) = prepared else {
            return false;
        };
        let _turn = self.inner.writes.lock().await;
        let service = self.service();
        let wanted = id.clone();
        let forgotten = field.to_owned();
        let result = blocking(move || {
            service.secret_delete(&wanted, &forgotten)?;
            service.view(&wanted)
        })
        .await;
        self.change(|state| {
            state.count(&id, -1);
            state.summaries = self.inner.handle.integrations().summaries();
            let Some(detail) = state.detail_of(&id, opened) else {
                return false;
            };
            detail.busy = None;
            match result {
                Ok(view) => {
                    detail.take(view, true);
                    detail.entered.remove(field);
                    true
                }
                Err(error) => {
                    let label = detail.label(field);
                    detail.notice = Some(
                        Notice::new(
                            NoticeKind::Error,
                            format!("Couldn't forget the {label}. {RESTART}"),
                            "forget-failed",
                        )
                        .with(describe(&error)),
                    );
                    false
                }
            }
        })
    }

    /// Starts a check, at most once per floor. The Companion answers before the check ends.
    pub async fn refresh(&self) -> bool {
        let prepared = self.change(|state| {
            let summary = state.summary(state.detail.as_ref()?.id.as_str())?.clone();
            let detail = state.detail.as_mut()?;
            let usable = summary.enabled && summary.status.state != "not_configured";
            if detail.busy.is_some() || !usable || !summary.actions.iter().any(|a| a == "refresh") {
                return None;
            }
            let now = Instant::now();
            if detail.refresh_after.is_some_and(|after| now < after) {
                detail.notice = Some(Notice::new(
                    NoticeKind::Info,
                    floor_text(&detail.id),
                    "refresh-floor",
                ));
                return None;
            }
            detail.busy = Some(Busy::Refresh);
            detail.notice = None;
            detail.refresh_after = Some(now + floor(&detail.id));
            Some((detail.id.clone(), detail.opened))
        });
        let Some((id, opened)) = prepared else {
            return false;
        };
        let _turn = self.inner.writes.lock().await;
        let service = self.service();
        let wanted = id.clone();
        let result = blocking(move || {
            service.refresh(&wanted)?;
            service.view(&wanted)
        })
        .await;
        self.change(|state| {
            let Some(detail) = state.detail_of(&id, opened) else {
                return false;
            };
            detail.busy = None;
            match result {
                Ok(view) => {
                    detail.take(view, true);
                    detail.notice = Some(Notice::new(NoticeKind::Ok, "Check started", "refreshed"));
                    true
                }
                Err(error) => {
                    detail.notice = Some(
                        Notice::new(
                            NoticeKind::Error,
                            format!("Couldn't check. {RESTART}"),
                            "refresh-failed",
                        )
                        .with(describe(&error)),
                    );
                    false
                }
            }
        })
    }

    /// Brings the approval dialog the custom Integration waits on to the front again. `false`
    /// when there is none, it was answered or expired, or it was shown within the last 5 s.
    pub async fn show_prompt(&self) -> bool {
        let Some(request) = self.view().detail.and_then(|d| d.approval) else {
            return false;
        };
        let actions = self.inner.handle.actions().clone();
        tokio::task::spawn_blocking(move || actions.show(&request.id).is_ok())
            .await
            .unwrap_or(false)
    }
}

fn floor(id: &str) -> Duration {
    if id == "custom" {
        CUSTOM_REFRESH_FLOOR
    } else {
        REFRESH_FLOOR
    }
}

/// The line under Check now.
pub fn floor_text(id: &str) -> &'static str {
    if id == "custom" {
        CUSTOM_FLOOR_TEXT
    } else {
        FLOOR_TEXT
    }
}

async fn follow(
    session: Weak<Inner>,
    mut changes: tokio::sync::broadcast::Receiver<Change>,
    mut closed: watch::Receiver<IntegrationsView>,
) {
    loop {
        tokio::select! {
            change = changes.recv() => {
                let Some(inner) = session.upgrade() else { return };
                let session = IntegrationsSession { inner };
                match change {
                    Ok(Change::Stopping) | Err(RecvError::Closed) => return,
                    Ok(change) => session.observe(&change),
                    Err(RecvError::Lagged(_)) => session.sync(),
                }
            }
            gone = closed.changed() => if gone.is_err() { return },
        }
    }
}
