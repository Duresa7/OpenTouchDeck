//! Integration descriptors and runtime contract. Modules register their factory once.
mod cliproxy;
mod custom;
pub mod media;
pub use custom::CommandApproval;
pub mod secrets;
pub mod this_pc;
pub mod volume;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use ts_rs::TS;

#[derive(Clone, Serialize, Deserialize, TS)]
pub struct ConfigField {
    pub name: String,
    pub kind: String,
    pub secret: bool,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct DataStream {
    pub id: String,
    pub kind: String,
    pub unit: Option<String>,
    /// A Device's `input` Tile can set it, through the Integration's `Control`.
    #[serde(default)]
    pub settable: bool,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct PresetTile {
    pub id: String,
    pub name: String,
    pub action: Value,
    /// How the Tile draws its stream, when not as a value with its label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<td_core::layout::TileRender>,
    /// The streams the Tile declares, by the renderer's stream id, when it binds more than its field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streams: Option<BTreeMap<String, td_core::layout::StreamType>>,
    /// The Tile takes a drag that sets its `value` stream.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub input: bool,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct Delivery {
    pub device: String,
    pub sent: u64,
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct IntegrationStatus {
    pub state: String,
    pub message: Option<String>,
    pub delivery: Vec<Delivery>,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct IntegrationSummary {
    pub revision: u64,
    pub id: String,
    pub name: String,
    pub icon: String,
    pub enabled: bool,
    pub status: IntegrationStatus,
    pub config_schema: Vec<ConfigField>,
    pub streams: Vec<DataStream>,
    pub actions: Vec<String>,
    pub presets: Vec<PresetTile>,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct SecretState {
    pub set: bool,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct IntegrationView {
    pub detail: Option<CliProxyDetail>,
    pub summary: IntegrationSummary,
    pub config: Value,
    pub secrets: BTreeMap<String, SecretState>,
}
#[derive(Debug)]
pub enum Error {
    NotFound,
    ConfigInvalid,
    ConfigField {
        field: &'static str,
        message: &'static str,
    },
    Credentials,
    Storage,
}

pub type PollState = Box<dyn std::any::Any + Send>;
pub type Poll = Box<dyn FnOnce() -> PollState + Send>;

pub trait Integration: Send {
    fn prepare_poll(&mut self) -> Option<Poll> {
        None
    }
    fn apply_poll(&mut self, _state: PollState) {}
    fn request_refresh(&mut self) {
        self.refresh();
    }
    fn command_approval(&mut self, _approval: Arc<dyn CommandApproval>) {}
    fn summary(&self) -> IntegrationSummary;
    fn view(&self) -> Result<IntegrationView, Error>;
    fn configure(&mut self, config: Value) -> Result<IntegrationView, Error>;
    /// Turns the Integration on or off and leaves its other settings and its secrets as stored.
    /// The default writes the stored config back with only `enabled` changed.
    fn set_enabled(&mut self, enabled: bool) -> Result<IntegrationView, Error> {
        let mut config = self.view()?.config;
        config
            .as_object_mut()
            .ok_or(Error::ConfigInvalid)?
            .insert("enabled".into(), Value::Bool(enabled));
        self.configure(config)
    }
    fn secret_put(&mut self, field: &str, value: &str) -> Result<(), Error>;
    fn secret_delete(&mut self, field: &str) -> Result<(), Error>;
    fn refresh(&mut self);
    fn tick(&mut self);
    /// Latest Live tile values, consumed by the Device scheduler, not the HTTP API.
    fn readings(&self) -> Vec<crate::sources::Reading>;
    /// The newest typed value of each stream, which the Companion sends to Devices whose Layouts bind it.
    fn live(&self) -> Vec<LiveSample> {
        Vec::new()
    }
    /// The Device service reports delivery independently of upstream polling.
    fn delivered(&mut self, delivery: Delivery);
    /// The handle Devices reach this Integration through without the registry lock. Built-in Integrations
    /// that act on the PC have one.
    fn control(&self) -> Option<Arc<dyn Control>> {
        None
    }
}
/// What a Device asks of an Integration, without the registry lock. It starts no process, and no API route
/// reaches it: only a Device's press `part` and `v2.input` do.
pub trait Control: Send + Sync {
    /// The newest samples, without blocking.
    fn live(&self) -> Vec<LiveSample>;
    /// Called once. The Integration calls `wake` when a sample changes, so delivery doesn't wait for a round.
    fn subscribe(&self, wake: Arc<dyn Fn() + Send + Sync>);
    /// Receives the press `part` unchanged as `action`, blocking at most `timeout`.
    fn act(&self, account: &str, action: &str, timeout: Duration) -> Result<(), ControlError>;
    /// Sets the Integration stream id bound by the Tile stream: for `value`, the widget
    /// `field` (`level` for Volume); other streams bind the same id. `settable` is checked
    /// on that Integration stream. Queues a Device's drag and returns without blocking.
    fn set(&self, account: &str, stream: &str, value: f64) -> Result<(), ControlError>;
    /// The source picture an `image` sample of this Control names.
    fn picture(&self, _id: &str) -> Option<Arc<[u8]>> {
        None
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    UnknownAccount,
    UnknownAction,
    NotSettable,
    OutOfRange,
    /// The OS backend can't do it now, in the words the action log shows.
    Unavailable(String),
}
/// The newest value of one Integration stream, for Live tiles. An `image` value names a source picture that
/// [`Control::picture`] holds; the Companion turns it into a cover Asset before any Device sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSample {
    pub account: String,
    pub stream: String,
    /// Rises with every new sample, also one equal to the last, because a graph advances on each.
    pub sample: u64,
    /// How long a Device draws the value before it shows it stale, 1000 to 60000.
    pub ttl_ms: u32,
    pub value: td_core::live::LiveValue,
}
pub struct Environment {
    pub media_provider: Option<Arc<dyn media::MediaProvider>>,
    pub volume: Option<volume::ProviderFactory>,
    pub secrets: Arc<dyn secrets::SecretStore>,
    pub config_file: Option<PathBuf>,
    pub fetch: Arc<dyn Fn() -> Box<dyn crate::sources::Fetch> + Send + Sync>,
}
pub struct Registration(pub fn(&Environment) -> Result<Box<dyn Integration>, Error>);
inventory::collect!(Registration);
pub struct Registry(BTreeMap<String, Box<dyn Integration>>);
impl Registry {
    pub fn new(env: Environment) -> Result<Self, Error> {
        Self::from_registrations(env, inventory::iter::<Registration>)
    }
    fn from_registrations<'a>(
        env: Environment,
        registrations: impl IntoIterator<Item = &'a Registration>,
    ) -> Result<Self, Error> {
        let mut entries = BTreeMap::new();
        for r in registrations {
            let i = (r.0)(&env)?;
            let summary = i.summary();
            secrets::validate_id(&summary.id).map_err(|_| Error::ConfigInvalid)?;
            for field in &summary.config_schema {
                secrets::validate_id(&field.name).map_err(|_| Error::ConfigInvalid)?;
            }
            if entries.insert(summary.id, i).is_some() {
                return Err(Error::ConfigInvalid);
            }
        }
        Ok(Self(entries))
    }
    pub fn controls(&self) -> BTreeMap<String, Arc<dyn Control>> {
        self.0
            .iter()
            .filter_map(|(id, i)| i.control().map(|c| (id.clone(), c)))
            .collect()
    }
    pub fn list(&self) -> Vec<IntegrationSummary> {
        self.0.values().map(|i| i.summary()).collect()
    }
    pub fn get(&mut self, id: &str) -> Result<&mut Box<dyn Integration>, Error> {
        self.0.get_mut(id).ok_or(Error::NotFound)
    }
    pub fn poll_jobs(&mut self, busy: &std::collections::BTreeSet<String>) -> Vec<(String, Poll)> {
        self.0
            .iter_mut()
            .filter(|(id, _)| !busy.contains(*id))
            .filter_map(|(id, i)| i.prepare_poll().map(|poll| (id.clone(), poll)))
            .collect()
    }
    pub fn tick(&mut self) {
        for i in self.0.values_mut() {
            i.tick();
        }
    }
    /// Each Integration's newest live samples, by Integration id.
    pub fn live(&self) -> impl Iterator<Item = (&str, Vec<LiveSample>)> {
        self.0.iter().map(|(id, i)| (id.as_str(), i.live()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_registration_is_rejected() {
        let env = Environment {
            media_provider: Some(Arc::new(media::fake::FakeMedia::default())),
            volume: Some(volume::fake::FakeVolume::default().factory()),
            secrets: Arc::new(secrets::MemorySecrets::default()),
            config_file: None,
            fetch: Arc::new(|| Box::new(crate::sources::HttpFetch::new())),
        };
        let registration = inventory::iter::<Registration>.into_iter().next().unwrap();
        assert!(Registry::from_registrations(env, [registration, registration]).is_err());
    }

    /// `UI_WIDGET_CAPTION_MAX` in `firmware/components/td_ui_core/include/ui_widget_state.h`.
    const FIRMWARE_CAPTION_MAX_BYTES: usize = 16;

    #[test]
    fn preset_captions_fit_the_firmware_caption_limit() {
        let env = Environment {
            media_provider: Some(Arc::new(media::fake::FakeMedia::default())),
            volume: Some(volume::fake::FakeVolume::default().factory()),
            secrets: Arc::new(secrets::MemorySecrets::default()),
            config_file: None,
            fetch: Arc::new(|| Box::new(crate::sources::HttpFetch::new())),
        };
        let registry = Registry::new(env).unwrap();
        let mut presets = 0;
        for (id, integration) in &registry.0 {
            for preset in integration.summary().presets {
                presets += 1;
                assert!(
                    preset.name.len() <= FIRMWARE_CAPTION_MAX_BYTES,
                    "{id} preset {:?} is {} bytes",
                    preset.name,
                    preset.name.len()
                );
            }
        }
        assert!(presets > 0);
    }

    #[test]
    fn polling_integrations_have_no_control() {
        let env = Environment {
            media_provider: Some(Arc::new(media::fake::FakeMedia::default())),
            volume: Some(volume::fake::FakeVolume::default().factory()),
            secrets: Arc::new(secrets::MemorySecrets::default()),
            config_file: None,
            fetch: Arc::new(|| Box::new(crate::sources::HttpFetch::new())),
        };
        let registry = Registry::new(env).unwrap();
        assert!(!registry.0.is_empty());
        for (id, integration) in &registry.0 {
            if id == "media" || id == "volume" {
                continue;
            }
            assert!(integration.control().is_none(), "{id}");
            let summary = integration.summary();
            assert!(summary.streams.iter().all(|s| !s.settable), "{id}");
            assert!(summary.presets.iter().all(|p| !p.input), "{id}");
        }
    }

    #[test]
    fn settable_and_media_preset_fields_are_optional_on_the_wire() {
        let stream: DataStream = serde_json::from_value(
            serde_json::json!({"id": "cpu", "kind": "numeric", "unit": "%"}),
        )
        .unwrap();
        assert!(!stream.settable);
        let preset = PresetTile {
            id: "volume".into(),
            name: "Volume".into(),
            action: serde_json::json!({"type": "widget", "source": "volume", "field": "level", "account": ""}),
            render: Some(td_core::layout::TileRender::Bar),
            streams: Some(BTreeMap::from([
                ("value".into(), td_core::layout::StreamType::Number),
                ("muted".into(), td_core::layout::StreamType::Number),
            ])),
            input: true,
        };
        let wire = serde_json::to_value(&preset).unwrap();
        assert_eq!(wire["render"], "bar");
        assert_eq!(wire["streams"]["muted"], "number");
        assert_eq!(wire["input"], true);
        let plain = PresetTile {
            render: None,
            streams: None,
            input: false,
            ..preset
        };
        let wire = serde_json::to_value(&plain).unwrap();
        assert!(wire.get("streams").is_none() && wire.get("input").is_none());
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct UsageWindow {
    pub pct: f64,
    pub resets_at_ms: Option<i64>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AccountUsage {
    pub provider: String,
    pub account: String,
    pub status: String,
    pub success: u64,
    pub failed: u64,
    pub recent_10m: u64,
    pub plan: Option<String>,
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub error: Option<String>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct CliProxyDetail {
    pub accounts: Vec<AccountUsage>,
    pub polled_ms: u64,
}
