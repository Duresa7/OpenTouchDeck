mod dialogs;
use crate::{
    api::{ApiError, CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req},
    devices::DeviceId,
    dialog_text::shown,
    jobs::{Job, JobError, JobKind, JobScope},
};
use axum::http::{Method, StatusCode};
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
use td_link::{
    bundle::Bundle,
    update::{self, UpdateState},
};
use ts_rs::TS;

const CONSENT_WAIT: Duration = Duration::from_secs(300);
#[cfg(any(test, feature = "test-hooks"))]
const TEST_BUNDLE: &[u8] = include_bytes!("../../../../td-link/tests/fixtures/touchdeck-test.tdfw");
#[derive(Clone, Debug)]
pub struct Confirmation {
    pub question: String,
    pub target: String,
    pub name: String,
    pub port: String,
    pub current: String,
    pub version: String,
    pub build: String,
}
impl Confirmation {
    pub fn text(&self) -> String {
        format!(
            "{}\nTarget: {} ({})\nPort: {}\nCurrent: {}\nBundle: {}\nBuild: {}",
            self.question,
            shown(&self.name),
            shown(&self.target),
            shown(&self.port),
            shown(&self.current),
            shown(&self.version),
            shown(&self.build)
        )
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
struct Picked {
    target: DeviceId,
    until: Instant,
    bundle: Arc<Bundle>,
}
#[derive(Clone)]
pub struct Firmware {
    picked: Arc<Mutex<HashMap<String, Picked>>>,
    dialog: Arc<AtomicBool>,
    prompter: Arc<Mutex<Arc<dyn Prompter>>>,
    bundled: Option<Arc<Bundle>>,
    #[cfg(any(test, feature = "test-hooks"))]
    scripted: Arc<Scripted>,
}
impl Firmware {
    pub(crate) fn new(mock: bool) -> Self {
        let bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundled.tdfw"));
        #[cfg(any(test, feature = "test-hooks"))]
        let bytes = if mock { TEST_BUNDLE } else { bytes };
        let _ = mock;
        Self {
            picked: Default::default(),
            dialog: Default::default(),
            prompter: Arc::new(Mutex::new(Arc::new(Native))),
            bundled: (!bytes.is_empty()).then(|| {
                Arc::new(Bundle::parse(bytes).expect("build.rs checked the bundled Firmware"))
            }),
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
    fn bundled(&self) -> Result<Arc<Bundle>, FirmwareError> {
        self.bundled.clone().ok_or(FirmwareError::Invalid)
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
impl ApiError for FirmwareError {
    const CODES: &'static [&'static str] = &[
        "firmware.bundle_invalid",
        "firmware.bundle_not_found",
        "new_board.not_found",
        "firmware.usb_only",
        "system.dialog_open",
        "firmware.declined",
        "firmware.failed",
        "firmware.rolled_back",
        "firmware.confirm_timeout",
    ];
    fn status(&self) -> StatusCode {
        match self {
            Self::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Missing | Self::NewBoardNotFound => StatusCode::NOT_FOUND,
            Self::Common(e) => e.status(),
            Self::Job(e) => e.status(),
            Self::Device(e) => e.status(),
            _ => StatusCode::CONFLICT,
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "firmware.bundle_invalid",
            Self::Missing => "firmware.bundle_not_found",
            Self::NewBoardNotFound => "new_board.not_found",
            Self::UsbOnly => "firmware.usb_only",
            Self::DialogOpen => "system.dialog_open",
            Self::Declined => "firmware.declined",
            Self::Failed(s) if s == "rolled back" => "firmware.rolled_back",
            Self::Failed(s) if s == "confirm timeout" => "firmware.confirm_timeout",
            Self::Failed(_) => "firmware.failed",
            Self::Common(e) => e.code(),
            Self::Job(e) => e.code(),
            Self::Device(e) => e.code(),
        }
    }
    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Failed(reason) => Some(serde_json::json!({"reason":reason})),
            Self::Job(e) => e.details(),
            Self::Device(e) => e.details(),
            _ => None,
        }
    }
}
#[derive(Serialize, Deserialize, TS)]
pub struct FirmwareBuild {
    pub version: String,
    pub build: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct BundleInfo {
    pub id: String,
    pub version: String,
    pub build: String,
    pub bytes: usize,
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
pub struct Bundled;
impl Endpoint for Bundled {
    const NAME: &'static str = "firmware.bundled";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/firmware/bundled";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = FirmwareBuild;
    type Error = FirmwareError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        let b = ctx.firmware.bundled()?;
        Ok(FirmwareBuild {
            version: b.manifest.version.clone(),
            build: b.manifest.elf_sha256.clone(),
        })
    }
}
pub struct Pick;
impl Endpoint for Pick {
    const NAME: &'static str = "firmware.pick";
    const METHOD: Method = Method::POST;
    const PATH: &'static str = "/firmware/bundles/pick";
    const UNTIMED: bool = true;
    type Path = ();
    type Query = NoQuery;
    type Body = FirmwareTarget;
    type Ok = PickResult;
    type Error = FirmwareError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        target(&ctx, &req.body.device, false)?;
        if ctx.firmware.dialog.swap(true, Ordering::SeqCst) {
            return Err(FirmwareError::DialogOpen);
        }
        let guard = Dialog(ctx.firmware.dialog.clone());
        let prompt = ctx.firmware.prompter.lock().clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let _input = ctx.devices.suppress_input();
            let Some(bytes) = prompt.pick().map_err(|_| FirmwareError::Invalid)? else {
                return Ok(PickResult { bundle: None });
            };
            let bundle = Arc::new(Bundle::parse(&bytes).map_err(|_| FirmwareError::Invalid)?);
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|_| CommonError::Internal)?;
            let id = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let info = BundleInfo {
                id: id.clone(),
                version: bundle.manifest.version.clone(),
                build: bundle.manifest.elf_sha256.clone(),
                bytes: bundle.app().len(),
            };
            let mut picked = ctx.firmware.picked.lock();
            picked.retain(|_, p| p.until > ctx.clock.now());
            if picked.len() >= 16 {
                return Err(CommonError::Limits("Too many selected Firmware files.").into());
            }
            picked.insert(
                id,
                Picked {
                    target: req.body.device,
                    until: ctx.clock.now() + Duration::from_secs(600),
                    bundle,
                },
            );
            Ok(PickResult { bundle: Some(info) })
        })
        .await
        .map_err(|_| CommonError::Internal)?
    }
}
fn target(
    ctx: &Ctx,
    id: &DeviceId,
    install: bool,
) -> Result<(String, String, String), FirmwareError> {
    if !install && let Some(d) = ctx.devices.get(id) {
        if d.carrier == Some(crate::devices::Carrier::Wifi) {
            return Err(FirmwareError::UsbOnly);
        }
        let port = ctx
            .devices
            .ports()
            .into_iter()
            .find(|p| &p.id == id)
            .ok_or(crate::link::DeviceLinkError::NotFound)?
            .port;
        return Ok((
            d.name.unwrap_or_else(|| id.to_string()),
            port,
            d.firmware
                .map(|f| f.version)
                .unwrap_or_else(|| "unknown".into()),
        ));
    }
    ctx.devices
        .new_boards()
        .into_iter()
        .find(|b| &b.id == id)
        .map(|b| (id.to_string(), b.port, "unknown".into()))
        .ok_or(if install {
            FirmwareError::NewBoardNotFound
        } else {
            crate::link::DeviceLinkError::NotFound.into()
        })
}
macro_rules! operation {
    ($name:ident, $kind:literal, $path:literal) => {
        pub struct $name;
        impl JobKind for $name {
            const NAME: &'static str = $kind;
            type Progress = FirmwareProgress;
            type Result = FirmwareResult;
        }
        impl Endpoint for $name {
            const NAME: &'static str = $kind;
            const METHOD: Method = Method::POST;
            const PATH: &'static str = $path;
            const STATUS: StatusCode = StatusCode::ACCEPTED;
            type Path = FirmwareTarget;
            type Query = NoQuery;
            type Body = FirmwareRequest;
            type Ok = Job;
            type Error = FirmwareError;
            async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Job, FirmwareError> {
                start::<Self>(ctx, req.path.device, req.body.bundle)
            }
        }
    };
}
operation!(
    Update,
    "firmware.update",
    "/devices/{device}/firmware/update"
);
operation!(
    Repair,
    "firmware.repair",
    "/devices/{device}/firmware/repair"
);
operation!(Install, "firmware.install", "/new-boards/{device}/install");
fn start<K: JobKind<Progress = FirmwareProgress, Result = FirmwareResult>>(
    ctx: Ctx,
    id: DeviceId,
    selection: String,
) -> Result<Job, FirmwareError> {
    let install = K::NAME == "firmware.install";
    let recovery = K::NAME != "firmware.update";
    let (name, port, current) = target(&ctx, &id, install)?;
    if !install && ctx.devices.get(&id).is_none() {
        return Err(crate::link::DeviceLinkError::NotFound.into());
    }

    let service = ctx.firmware.clone();
    let mut picked = service.picked.lock();
    let bundle = if selection == "bundled" {
        service.bundled()?
    } else {
        picked
            .get(&selection)
            .filter(|p| p.target == id && p.until > ctx.clock.now())
            .ok_or(FirmwareError::Missing)?
            .bundle
            .clone()
    };
    let consent = recovery || !same_or_newer(&bundle.manifest.version, &current);
    let question = if install {
        "Install Firmware on this New board?"
    } else if recovery {
        "Repair this Device's Firmware?"
    } else if older(&bundle.manifest.version, &current) {
        "Downgrade this Device's Firmware?"
    } else {
        "Replace this Device's Firmware with a version that may be older?"
    };
    let confirmation = Confirmation {
        question: question.into(),
        target: id.to_string(),
        name,
        port,
        current,
        version: bundle.manifest.version.clone(),
        build: bundle.manifest.elf_sha256.clone(),
    };
    let jobs = ctx.jobs.clone();
    let scope = if install {
        JobScope::NewBoard(id.to_string())
    } else {
        JobScope::Device(id.to_string())
    };
    if let Some(job) = jobs.holding(&scope) {
        return Err(JobError::Busy(Box::new(job)).into());
    }
    if !recovery {
        ctx.links.connected(&id)?;
    }
    let job = jobs.start::<K>(scope, true, move |job| {
        let hold = ctx
            .devices
            .firmware_hold(&id, job.id())
            .map_err(|_| FirmwareError::Failed("Target disconnected.".into()))?;
        if consent {
            job.waiting_for_confirmation();
            let _input = ctx.devices.suppress_input();
            let prompt = ctx.firmware.prompter.lock().clone();
            let deadline = ctx.clock.now() + CONSENT_WAIT;
            let expired = || ctx.clock.now() >= deadline;
            let accepted = prompt.confirm(&confirmation, &|| expired() || job.check().is_err());
            job.check()?;
            if !accepted || expired() {
                return Err(FirmwareError::Declined.into());
            }
        }
        job.set_cancellable(false);
        job.check()?;
        job.running();
        let mut emit = |s| {
            let (phase, pct) = match s {
                UpdateState::Checking => ("checking", None),
                UpdateState::Sending { pct } => ("sending", Some(pct)),
                UpdateState::Rebooting => ("rebooting", None),
                UpdateState::Confirming => ("confirming", None),
                UpdateState::Recovering { pct } => ("recovering", Some(pct)),
                _ => return,
            };
            job.progress(&FirmwareProgress {
                phase: phase.into(),
                pct,
                bytes: pct.map_or(0, |p| {
                    let total = if recovery {
                        bundle.images.iter().map(|image| image.data.len()).sum()
                    } else {
                        bundle.app().len()
                    };
                    total * usize::from(p) / 100
                }),
            });
        };
        let mut connector = hold.connector();
        let result = if recovery {
            if let Some(s) = &hold.session {
                s.close();
            }
            #[cfg(any(test, feature = "test-hooks"))]
            if let Some((ports, _)) = ctx.devices.mock() {
                let deck = ports
                    .deck(&id)
                    .ok_or_else(|| FirmwareError::Failed("Target disconnected.".into()))?;
                deck.flash_bundle(&bundle);
                emit(UpdateState::Recovering { pct: 100 });
                if install {
                    hold.installed();
                }
                return Ok(FirmwareResult {
                    message: "Firmware installed".into(),
                    version: bundle.manifest.version.clone(),
                });
            }
            let mut flasher = td_link::recover::RomRecovery(&hold.port);
            update::run(
                &mut *connector,
                Some(&mut flasher),
                &bundle,
                update::Deadlines::default(),
                &mut emit,
            )
        } else {
            let session = hold
                .session
                .as_ref()
                .ok_or_else(|| FirmwareError::Failed("Target disconnected.".into()))?;
            update::run_session(
                session,
                &mut *connector,
                &bundle,
                update::Deadlines::default(),
                &mut emit,
            )
        };
        result.map_err(FirmwareError::Failed)?;
        if install {
            hold.installed();
        }
        Ok(FirmwareResult {
            message: "Firmware updated".into(),
            version: bundle.manifest.version.clone(),
        })
    })?;
    if selection != "bundled" {
        picked.remove(&selection);
    }
    Ok(job)
}
fn versions(next: &str, current: &str) -> Option<(semver::Version, semver::Version)> {
    Some((
        semver::Version::parse(next.trim_start_matches('v')).ok()?,
        semver::Version::parse(current.trim_start_matches('v')).ok()?,
    ))
}
fn same_or_newer(next: &str, current: &str) -> bool {
    next == current || versions(next, current).is_some_and(|(n, c)| n >= c)
}
fn older(next: &str, current: &str) -> bool {
    versions(next, current).is_some_and(|(n, c)| n < c)
}
fn feature() -> Feature {
    Feature::new("firmware")
        .endpoint::<Bundled>()
        .endpoint::<Pick>()
        .endpoint::<Update>()
        .endpoint::<Repair>()
        .endpoint::<Install>()
        .job::<Update>()
        .job::<Repair>()
        .job::<Install>()
}
inventory::submit! { Registration(feature) }

#[cfg(test)]
mod tests {
    use super::{older, same_or_newer};

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
