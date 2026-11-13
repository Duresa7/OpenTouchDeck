//! Firmware owns selection, target checks, native consent and execution through completion.
//! Access it through `companion::Handle::firmware`; the local web API is only an adapter.
mod api;
mod dialogs;
mod task;
mod workflow;
use crate::{
    api::CommonError,
    devices::{DeviceId, FirmwareChannel},
    dialog_text::shown,
    jobs::{Job, JobError, JobKind, JobScope},
};
pub use api::{Bundled, Pick};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub use task::{
    BoardStep, BuildChoice, Context, FirmwareTask, JobView, Source, Target, TaskView, Words,
    board_step, build_choice, compare_versions, offered, short_version, view_of,
};
use td_link::bundle::Bundle;
use ts_rs::TS;
pub use workflow::Operation;

const CONSENT_WAIT: Duration = Duration::from_secs(300);
#[cfg(any(test, feature = "test-hooks"))]
const TEST_BUNDLE: &[u8] = include_bytes!("../../../../td-link/tests/fixtures/touchdeck-test.tdfw");
#[cfg(any(test, feature = "test-hooks"))]
const TEST_RADIO_BUNDLE: &[u8] =
    include_bytes!("../../../../td-link/tests/fixtures/touchdeck-test-radio.tdfw");
#[derive(Clone, Debug)]
pub struct Confirmation {
    pub question: String,
    pub target: String,
    pub name: String,
    pub port: String,
    pub current: String,
    pub version: String,
    pub build: String,
    /// The build the bundle is: the normal build or the radio build.
    pub channel: FirmwareChannel,
}
impl Confirmation {
    pub fn text(&self) -> String {
        format!(
            "{}\nTarget: {} ({})\nPort: {}\nCurrent: {}\nBundle: {} ({} build)\nBuild: {}",
            self.question,
            shown(&self.name),
            shown(&self.target),
            shown(&self.port),
            shown(&self.current),
            shown(&self.version),
            channel_word(self.channel),
            shown(&self.build)
        )
    }
}
/// What the consent dialog asks. It always says when the result changes the build the Device runs, or may change it.
fn question(
    install: bool,
    recovery: bool,
    running: Option<FirmwareChannel>,
    channel: FirmwareChannel,
    downgrade: bool,
) -> String {
    use FirmwareChannel::{Normal, Radio};
    const ADDS: &str = "It adds Wi-Fi and Bluetooth, which stay off until you set them up.";
    const UNUSED: &str = "Its Wi-Fi and Bluetooth setup stays stored but unused.";
    if install {
        return match channel {
            Radio => "Install the radio build on this New board?".into(),
            Normal => "Install Firmware on this New board?".into(),
        };
    }
    if recovery {
        let change = match (running, channel) {
            (Some(Radio), Normal) => {
                format!(" This installs the normal build instead of the radio build. {UNUSED}")
            }
            (Some(Normal), Radio) => {
                format!(" This installs the radio build instead of the normal build. {ADDS}")
            }
            (None, Normal) => " This installs the normal build. If the Device ran the radio build, it comes \
                 back on the normal build, and its Wi-Fi and Bluetooth setup stays stored but unused."
                .into(),
            (None, Radio) => format!(" This installs the radio build. {ADDS}"),
            _ => String::new(),
        };
        return format!("Repair this Device's Firmware?{change}");
    }
    match (running.is_some_and(|running| running != channel), channel) {
        (true, Radio) => format!("Switch this Device to the radio build? {ADDS}"),
        (true, Normal) => format!("Switch this Device back to the normal build? {UNUSED}"),
        _ if downgrade => "Downgrade this Device's Firmware?".into(),
        _ => "Replace this Device's Firmware with a version that may be older?".into(),
    }
}
fn channel_word(channel: FirmwareChannel) -> &'static str {
    match channel {
        FirmwareChannel::Normal => "normal",
        FirmwareChannel::Radio => "radio",
    }
}
pub trait Prompter: Send + Sync {
    fn pick(&self) -> Result<Option<Vec<u8>>, String>;
    /// Shows the consent dialog. Must close it and return once `closed` turns true.
    fn confirm(&self, confirmation: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool;
}
struct Native;
impl Prompter for Native {
    fn pick(&self) -> Result<Option<Vec<u8>>, String> {
        use std::io::Read;
        dialogs::pick()?
            .map(|path| {
                let file = std::fs::File::open(path)
                    .map_err(|_| "Could not read Firmware.".to_string())?;
                let mut bytes = Vec::new();
                file.take(5 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "Could not read Firmware.".to_string())?;
                if bytes.len() > 5 * 1024 * 1024 {
                    return Err("Firmware file is too large.".into());
                }
                Ok(bytes)
            })
            .transpose()
    }
    fn confirm(&self, confirmation: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool {
        dialogs::confirm(confirmation, closed)
    }
}
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Serialize, Deserialize, Clone, Copy, TS)]
#[serde(rename_all = "lowercase")]
pub enum ConsentAnswer {
    Accept,
    Decline,
}
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Default)]
struct Scripted(Mutex<std::collections::VecDeque<ConsentAnswer>>);
#[cfg(any(test, feature = "test-hooks"))]
impl Prompter for Scripted {
    fn pick(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(TEST_BUNDLE.to_vec()))
    }
    fn confirm(&self, _: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool {
        loop {
            if closed() {
                return false;
            }
            if let Some(answer) = self.0.lock().pop_front() {
                return matches!(answer, ConsentAnswer::Accept);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
/// The Firmware this Companion carries. A build made without the bundle variables carries none.
#[derive(Default)]
struct Bundles {
    normal: Option<Arc<Bundle>>,
    radio: Option<Arc<Bundle>>,
}
struct Picked {
    target: DeviceId,
    until: Instant,
    bundle: Arc<Bundle>,
}
#[derive(Clone)]
pub struct Firmware {
    devices: crate::devices::DeviceManager,
    links: crate::link::Links,
    jobs: crate::jobs::Jobs,
    clock: crate::clock::Clock,
    /// A new install asks for setup again, so it clears the board's old setup skip.
    skips: crate::setup::Skips,
    picked: Arc<Mutex<HashMap<String, Picked>>>,
    dialog: Arc<AtomicBool>,
    prompter: Arc<Mutex<Arc<dyn Prompter>>>,
    /// The normal build and, when it carries one, the radio build. Fixed at build time; only tests change it.
    bundles: Arc<Mutex<Bundles>>,
    #[cfg(any(test, feature = "test-hooks"))]
    scripted: Arc<Scripted>,
}
impl Firmware {
    pub(crate) fn new(
        mock: bool,
        devices: crate::devices::DeviceManager,
        links: crate::link::Links,
        jobs: crate::jobs::Jobs,
        clock: crate::clock::Clock,
        skips: crate::setup::Skips,
    ) -> Self {
        let bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundled.tdfw"));
        let radio: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundled-radio.tdfw"));
        #[cfg(any(test, feature = "test-hooks"))]
        let (bytes, radio) = if mock {
            (TEST_BUNDLE, TEST_RADIO_BUNDLE)
        } else {
            (bytes, radio)
        };
        let _ = mock;
        let parse = |bytes: &[u8]| {
            (!bytes.is_empty()).then(|| {
                Arc::new(Bundle::parse(bytes).expect("build.rs checked the bundled Firmware"))
            })
        };
        Self {
            devices,
            links,
            jobs,
            clock,
            skips,
            picked: Default::default(),
            dialog: Default::default(),
            prompter: Arc::new(Mutex::new(Arc::new(Native))),
            bundles: Arc::new(Mutex::new(Bundles {
                normal: parse(bytes),
                radio: parse(radio),
            })),
            #[cfg(any(test, feature = "test-hooks"))]
            scripted: Default::default(),
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_prompter(&self, prompter: Arc<dyn Prompter>) {
        *self.prompter.lock() = prompter;
    }
    /// Replaces the native dialogs with a prompter that gives these answers in order and waits once
    /// they run out.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn script(&self, answers: Vec<ConsentAnswer>) {
        *self.scripted.0.lock() = answers.into();
        self.set_prompter(self.scripted.clone());
    }
    /// Replaces the Firmware this Companion carries, as a build made with other bundle variables would. `None` carries
    /// no bundle for that channel, like a release built without `TOUCHDECK_FIRMWARE_BUNDLE`.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_bundles(&self, normal: Option<&[u8]>, radio: Option<&[u8]>) {
        let parse = |bytes: &[u8]| Arc::new(Bundle::parse(bytes).expect("a test bundle parses"));
        *self.bundles.lock() = Bundles {
            normal: normal.map(parse),
            radio: radio.map(parse),
        };
    }
    /// Whether this Companion carries a bundle of this build.
    pub fn carries(&self, channel: FirmwareChannel) -> bool {
        self.bundled(channel).is_ok()
    }
    /// The bundle this Companion carries for a channel. build.rs refused a bundle of the other build, and this checks the
    /// mark again, so a radio image can never be the normal channel's bundle.
    fn bundled(&self, channel: FirmwareChannel) -> Result<Arc<Bundle>, FirmwareError> {
        let bundles = self.bundles.lock();
        match channel {
            FirmwareChannel::Normal => &bundles.normal,
            FirmwareChannel::Radio => &bundles.radio,
        }
        .clone()
        .filter(|bundle| bundle.manifest.radio == channel.is_radio())
        .ok_or(FirmwareError::Invalid)
    }
}
struct Dialog(Arc<AtomicBool>);
impl Drop for Dialog {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
#[derive(Debug, thiserror::Error)]
pub enum FirmwareError {
    #[error("The Firmware bundle is invalid or unavailable.")]
    Invalid,
    #[error("The bundle expired, was consumed, or belongs to another target.")]
    Missing,
    #[error("A native file dialog is already open.")]
    DialogOpen,
    #[error("Firmware confirmation was declined.")]
    Declined,
    #[error("The bundle is the {} build, not the {} build.", channel_word(*.bundle), channel_word(*.channel))]
    ChannelMismatch {
        channel: FirmwareChannel,
        bundle: FirmwareChannel,
    },
    #[error("No New board with that id.")]
    NewBoardNotFound,
    #[error("Firmware updates need the Device on USB. Connect it with a cable and try again.")]
    UsbOnly,
    #[error("Firmware failed: {0}")]
    Failed(String),
    #[error(transparent)]
    Common(#[from] CommonError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error(transparent)]
    Device(#[from] crate::link::DeviceLinkError),
}
/// The Firmware this Companion carries: the normal build, and the radio build when it carries one.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct FirmwareBuild {
    pub version: String,
    pub build: String,
    pub radio: Option<RadioBuild>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RadioBuild {
    pub version: String,
    pub build: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct BundleInfo {
    pub id: String,
    pub version: String,
    pub build: String,
    pub bytes: usize,
    pub channel: FirmwareChannel,
}
#[derive(Serialize, Deserialize, TS)]
pub struct PickResult {
    pub bundle: Option<BundleInfo>,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FirmwareTarget {
    pub device: DeviceId,
}
#[derive(Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FirmwareRequest {
    pub bundle: String,
    /// The build to install. Left out, it is the build the Device runs, or the normal build on a New board. Naming the
    /// other build is how the person asks to switch, and a switch always asks for consent. A bundle of any other build
    /// is refused with `firmware.channel_mismatch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub channel: Option<FirmwareChannel>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct FirmwareProgress {
    pub phase: String,
    pub pct: Option<u8>,
    pub bytes: usize,
}
#[derive(Serialize, Deserialize, TS)]
pub struct FirmwareResult {
    pub message: String,
    pub version: String,
}

// Typed Job markers are shared by the workflow and the web adapter.
macro_rules! job_kind {
    ($name:ident, $kind:literal) => {
        pub struct $name;
        impl JobKind for $name {
            const NAME: &'static str = $kind;
            type Progress = FirmwareProgress;
            type Result = FirmwareResult;
        }
    };
}
job_kind!(Update, "firmware.update");
job_kind!(Repair, "firmware.repair");
job_kind!(Install, "firmware.install");

#[cfg(test)]
mod tests {
    use super::{
        FirmwareChannel, question,
        workflow::{older, same_or_newer},
    };

    #[test]
    fn a_repair_says_when_it_changes_the_build_or_may_change_it() {
        use FirmwareChannel::{Normal, Radio};
        let repair = |running, channel| question(false, true, running, channel, false);
        assert_eq!(
            repair(Some(Normal), Normal),
            "Repair this Device's Firmware?"
        );
        assert_eq!(repair(Some(Radio), Radio), "Repair this Device's Firmware?");
        assert_eq!(
            repair(Some(Radio), Normal),
            "Repair this Device's Firmware? This installs the normal build instead of the radio build. \
             Its Wi-Fi and Bluetooth setup stays stored but unused."
        );
        assert_eq!(
            repair(Some(Normal), Radio),
            "Repair this Device's Firmware? This installs the radio build instead of the normal build. \
             It adds Wi-Fi and Bluetooth, which stay off until you set them up."
        );
        assert_eq!(
            repair(None, Normal),
            "Repair this Device's Firmware? This installs the normal build. If the Device ran the radio build, \
             it comes back on the normal build, and its Wi-Fi and Bluetooth setup stays stored but unused."
        );
        assert!(repair(None, Radio).contains("This installs the radio build."));
    }

    #[test]
    fn semantic_versions_require_consent_only_for_downgrades() {
        assert!(same_or_newer("v2.0.0", "1.0.0"));
        assert!(same_or_newer("1.0.0", "1.0.0"));
        assert!(!same_or_newer("1.0.0", "2.0.0"));
        assert!(!same_or_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!same_or_newer("unknown", "1.0.0"));
    }

    #[test]
    fn only_ordered_lower_versions_read_as_downgrades() {
        assert!(older("1.0.0", "v2.0.0"));
        assert!(older("1.0.0-rc.1", "1.0.0"));
        assert!(!older("2.0.0", "1.0.0"));
        assert!(!older("1.0.0", "1.0.0"));
        assert!(!older("unknown", "1.0.0"));
    }
}
