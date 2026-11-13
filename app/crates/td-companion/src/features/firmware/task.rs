//! A Firmware Job as a screen follows it: one install on a New board or one update of a Device,
//! with the words for each step. Ported from the 1.x app's `lib/device/firmware.ts` and
//! `new-board.ts`, so setup and Settings word progress and failures the same way.
use super::{FirmwareBuild, FirmwareError, FirmwareRequest, Operation, PickResult};
use crate::{
    api::{ApiError, ErrorBody},
    companion::{Change, Handle, RecvError},
    devices::{DeviceId, DeviceSummary, FirmwareChannel},
    jobs::{Job, JobError, JobState},
};
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::watch;

/// What a Firmware Job is doing, in words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum JobView {
    #[default]
    Idle,
    /// The file dialog is open on this PC.
    Choosing {
        text: String,
    },
    /// The consent dialog is open on this PC.
    Waiting {
        text: String,
    },
    Running {
        text: String,
        pct: Option<u8>,
    },
    Done {
        text: String,
        version: String,
    },
    Failed {
        text: String,
        code: String,
    },
    Cancelled {
        text: String,
    },
}
impl JobView {
    pub fn in_progress(&self) -> bool {
        matches!(
            self,
            Self::Choosing { .. } | Self::Waiting { .. } | Self::Running { .. }
        )
    }
    pub fn text(&self) -> &str {
        match self {
            Self::Idle => "",
            Self::Choosing { text }
            | Self::Waiting { text }
            | Self::Running { text, .. }
            | Self::Done { text, .. }
            | Self::Failed { text, .. }
            | Self::Cancelled { text } => text,
        }
    }
}

/// A Job's view and the raw steps it went through, for Details.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskView {
    pub view: JobView,
    pub log: Vec<String>,
}

/// Which words a task uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Words {
    /// Updating a Device.
    Update,
    /// Installing Firmware on a New board.
    Install,
}

const WAITING: &str = "Waiting for confirmation on this PC.";
const CHOOSING: &str = "Choose a Firmware file on this PC…";
/// This Companion carries no bundle of the build asked for. A task-level code, never an API one.
pub const NOT_BUNDLED: &str = "firmware.not_bundled";
/// What a Device page says when this copy of Open TouchDeck carries no Firmware.
pub const NOT_BUNDLED_TEXT: &str =
    "This copy of Open TouchDeck doesn't include Firmware. Update from a file instead.";
pub const DECLINED: &str = "Declined on this PC. Nothing changed on the board.";
pub const INSTALL_FAILED: &str =
    "Couldn't install Firmware. Unplug the board, plug it back in, then retry.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Checking,
    Sending,
    Rebooting,
    Confirming,
    Recovering,
}
fn progress_of(value: &Value) -> Option<(Phase, Option<u8>)> {
    let phase = match value.get("phase")?.as_str()? {
        "checking" => Phase::Checking,
        "sending" => Phase::Sending,
        "rebooting" => Phase::Rebooting,
        "confirming" => Phase::Confirming,
        "recovering" => Phase::Recovering,
        _ => return None,
    };
    let pct = value
        .get("pct")
        .and_then(Value::as_u64)
        .map(|pct| pct.min(100) as u8);
    Some((phase, pct))
}
fn raw(phase: Phase) -> &'static str {
    match phase {
        Phase::Checking => "Checking",
        Phase::Sending => "Sending",
        Phase::Rebooting => "Rebooting",
        Phase::Confirming => "Confirming",
        Phase::Recovering => "Recovering",
    }
}

/// Who a task is about, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub device: String,
    pub version: String,
    /// The build a switch installs. `None` for an update within the build the Device runs.
    pub switch_to: Option<FirmwareChannel>,
}

fn reason(details: Option<&Value>) -> &str {
    details
        .and_then(|d| d.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("")
}
fn channel_word(channel: FirmwareChannel) -> &'static str {
    match channel {
        FirmwareChannel::Normal => "normal",
        FirmwareChannel::Radio => "radio",
    }
}
fn channel_in(details: Option<&Value>, key: &str) -> &'static str {
    match details.and_then(|d| d.get(key)).and_then(Value::as_str) {
        Some("radio") => "radio",
        _ => "normal",
    }
}

impl Words {
    fn progress(self, progress: Option<(Phase, Option<u8>)>) -> String {
        match self {
            Words::Install => match progress {
                Some((Phase::Sending | Phase::Recovering, Some(pct))) => {
                    format!("Installing Firmware… {pct}%")
                }
                _ => "Installing Firmware…".into(),
            },
            Words::Update => {
                let Some((phase, pct)) = progress else {
                    return "Updating…".into();
                };
                let pct = pct.unwrap_or(0);
                match phase {
                    Phase::Checking => "Checking the file…".into(),
                    Phase::Sending => format!("Updating… {pct}%"),
                    Phase::Recovering => format!("Repairing… {pct}%"),
                    Phase::Rebooting => "Restarting…".into(),
                    Phase::Confirming => "Checking that it started…".into(),
                }
            }
        }
    }
    fn done(self, message: &str, version: &str, context: &Context) -> String {
        match self {
            Words::Install => "Firmware installed".into(),
            Words::Update if context.switch_to.is_some() => format!(
                "Now on the {} build, {version}",
                channel_word(context.switch_to.unwrap_or_default())
            ),
            Words::Update => match message {
                "Firmware updated" => format!("Updated to {version}"),
                "Already on this build" => format!("Already on Firmware {version}"),
                message => format!("Done. {message}."),
            },
        }
    }
    /// The words for a failure, from its error code and details.
    pub fn failed(self, code: &str, details: Option<&Value>, context: &Context) -> String {
        if self == Words::Install {
            return match code {
                "firmware.declined" => DECLINED.into(),
                NOT_BUNDLED => {
                    "This copy of Open TouchDeck doesn't include Firmware to install.".into()
                }
                _ => INSTALL_FAILED.into(),
            };
        }
        let Context {
            device, version, ..
        } = context;
        match code {
            NOT_BUNDLED if context.switch_to.is_some() => format!(
                "This copy of Open TouchDeck doesn't include the {} build.",
                channel_in(details, "channel")
            ),
            NOT_BUNDLED if channel_in(details, "channel") == "radio" => {
                "This copy of Open TouchDeck doesn't include the radio build. Update from a file instead."
                    .into()
            }
            NOT_BUNDLED => NOT_BUNDLED_TEXT.into(),
            "firmware.rolled_back" => format!(
                "The new Firmware didn't start, so {device} went back to {version}. It still works. Retry, or repair it from Troubleshooting."
            ),
            "firmware.confirm_timeout" => format!(
                "{device} didn't come back after restarting. Unplug it and plug it back in. If its screen stays blank, repair it from Troubleshooting."
            ),
            "firmware.declined" => format!("Declined on this PC. {device} still runs {version}."),
            "firmware.channel_mismatch" => format!(
                "That file is the {} build, and {device} runs the {} build. Nothing changed.",
                channel_in(details, "bundle"),
                channel_in(details, "channel")
            ),
            "firmware.bundle_invalid" => "That file isn't a Firmware file for this Device.".into(),
            "firmware.bundle_not_found" => "That file expired. Choose it again, then retry.".into(),
            "system.dialog_open" => {
                "A file dialog is already open on this PC. Close it, then retry.".into()
            }
            "device.busy" => format!("{device} is updating. Wait for it to finish."),
            "device.not_connected" => format!("{device} isn't connected."),
            "job.cancelled" => format!("Update stopped. {device} still runs {version}."),
            "firmware.failed" if reason(details).contains("unplugged") => format!(
                "{device} was unplugged during the update. Plug it back in. If its screen stays blank, repair it from Troubleshooting."
            ),
            "firmware.failed" if reason(details).contains("too many chunks") => {
                "That file is over 6 MB, so it isn't a Firmware file.".into()
            }
            _ => format!(
                "Couldn't update. {device} still runs {version}. Check the cable, then retry."
            ),
        }
    }
}

/// A Job in words, with the raw step for Details.
pub fn view_of(job: &Job, context: &Context, words: Words) -> (JobView, String) {
    match job.state {
        JobState::Queued => (
            JobView::Running {
                text: words.progress(None),
                pct: None,
            },
            "Queued".into(),
        ),
        JobState::WaitingForConfirmation => (
            JobView::Waiting {
                text: WAITING.into(),
            },
            "Waiting for confirmation".into(),
        ),
        JobState::Running => {
            let progress = progress_of(&job.progress);
            let pct = progress.and_then(|(_, pct)| pct);
            let raw = match progress {
                Some((phase, Some(pct))) => format!("{} {pct}%", raw(phase)),
                Some((phase, None)) => raw(phase).into(),
                None => "Running".into(),
            };
            (
                JobView::Running {
                    text: words.progress(progress),
                    pct,
                },
                raw,
            )
        }
        JobState::Succeeded => {
            let result = job.result.as_ref();
            let field = |key| result.and_then(|r| r.get(key)).and_then(Value::as_str);
            let (message, version) = match (field("message"), field("version")) {
                (Some(message), Some(version)) => (message.to_owned(), version.to_owned()),
                _ => ("Firmware updated".to_owned(), context.version.clone()),
            };
            (
                JobView::Done {
                    text: words.done(&message, &version, context),
                    version,
                },
                message,
            )
        }
        JobState::Failed => {
            let error = job.error.clone().unwrap_or(ErrorBody {
                code: "internal".into(),
                message: "The Job ended as failed.".into(),
                details: None,
            });
            (
                JobView::Failed {
                    text: words.failed(&error.code, error.details.as_ref(), context),
                    code: error.code.clone(),
                },
                format!("Failed: {}", error.message),
            )
        }
        JobState::Cancelled => (
            JobView::Cancelled {
                text: words.failed("job.cancelled", None, context),
            },
            "Cancelled".into(),
        ),
    }
}

/// Where a New board's install is, as its card shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardStep {
    Detected,
    Waiting,
    Installing,
    Done,
    Declined,
    Failed,
}
pub fn board_step(view: &JobView) -> BoardStep {
    match view {
        JobView::Idle | JobView::Cancelled { .. } => BoardStep::Detected,
        JobView::Choosing { .. } | JobView::Waiting { .. } => BoardStep::Waiting,
        JobView::Running { .. } => BoardStep::Installing,
        JobView::Done { .. } => BoardStep::Done,
        JobView::Failed { code, .. } if code == "firmware.declined" => BoardStep::Declined,
        JobView::Failed { .. } => BoardStep::Failed,
    }
}

fn parts(version: &str) -> Option<[u64; 3]> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let mut numbers = version.splitn(3, '.');
    let number = |part: Option<&str>| -> Option<u64> {
        let digits: String = part?.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let major = number(numbers.next())?;
    let minor_part = numbers.next()?;
    let minor_digits: String = minor_part
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let minor = minor_digits.parse().ok()?;
    // A patch number only counts right after "major.minor.".
    let patch = if minor_digits.len() == minor_part.len() {
        number(numbers.next()).unwrap_or(0)
    } else {
        0
    };
    Some([major, minor, patch])
}

/// Compares two versions by major, minor and patch. `None` when either isn't a version.
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    Some(parts(a)?.cmp(&parts(b)?))
}

/// "1.4" for "1.4.2".
pub fn short_version(version: &str) -> String {
    parts(version).map_or_else(|| version.into(), |[a, b, _]| format!("{a}.{b}"))
}

/// The newer Firmware this Companion offers a Device: always of the build it runs.
pub fn offered(device: &DeviceSummary, builds: Option<&FirmwareBuild>) -> Option<String> {
    let firmware = device.firmware.as_ref()?;
    let Some(builds) = builds else {
        return firmware.update_available.clone();
    };
    let bundled = match firmware.channel {
        FirmwareChannel::Normal => Some(builds.version.as_str()),
        FirmwareChannel::Radio => builds.radio.as_ref().map(|r| r.version.as_str()),
    }?;
    (compare_versions(bundled, &firmware.version) == Some(std::cmp::Ordering::Greater))
        .then(|| bundled.to_owned())
}

/// The Firmware build row on a Device's page: the build it runs and the switch to the other one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildChoice {
    /// The build the Device runs now.
    pub current: FirmwareChannel,
    /// The build a switch installs.
    pub target: FirmwareChannel,
    /// "Normal build" or "Radio build".
    pub title: String,
    pub note: String,
    /// The button's words.
    pub action: String,
    /// The question before the switch, in the window, before the native consent.
    pub confirm_title: String,
    pub confirm_note: String,
    /// The confirm button's words.
    pub confirm_action: String,
    /// Switching back leaves the radio setup stored but unused, so the note warns.
    pub warn: bool,
}

const TURNS_ON: &str = "Wi-Fi and Bluetooth, both off until you set them up";

/// The build switch for a Device, or `None` when there is nothing to switch to: only when this Companion carries the
/// other build, and only for a Device that reports its Firmware.
pub fn build_choice(
    device: &DeviceSummary,
    builds: Option<&FirmwareBuild>,
    name: &str,
) -> Option<BuildChoice> {
    let current = device.firmware.as_ref()?.channel;
    let builds = builds?;
    Some(match current {
        // `builds` always holds the normal build.
        FirmwareChannel::Radio => BuildChoice {
            current,
            target: FirmwareChannel::Normal,
            title: "Radio build".into(),
            note: format!("Adds {TURNS_ON}. The normal build has no radio code."),
            action: "Switch to the normal build".into(),
            confirm_title: format!("Switch {name} back to the normal build?"),
            confirm_note: format!(
                "The Wi-Fi and Bluetooth setup stays stored on {name} but unused. {name} restarts once."
            ),
            confirm_action: "Switch".into(),
            warn: true,
        },
        FirmwareChannel::Normal => {
            builds.radio.as_ref()?;
            BuildChoice {
                current,
                target: FirmwareChannel::Radio,
                title: "Normal build".into(),
                note: format!("The radio build adds {TURNS_ON}. You can switch back any time."),
                action: "Install the radio build".into(),
                confirm_title: format!("Install the radio build on {name}?"),
                confirm_note: format!("It adds {TURNS_ON}. {name} restarts once."),
                confirm_action: "Install".into(),
                warn: false,
            }
        }
    })
}

/// Where a task's Firmware comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The Firmware this Companion carries.
    Bundled,
    /// A `.tdfw` file the person picks in the native file dialog on this PC.
    File,
}

/// What a task acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Device(DeviceId),
    NewBoard(DeviceId),
}

struct Inner {
    handle: Handle,
    target: Target,
    words: Words,
    view: watch::Sender<TaskView>,
    /// The Job followed now; a newer start replaces it.
    job: Mutex<Option<String>>,
    context: Mutex<Context>,
}

/// One Firmware Job for a screen: start it, follow it to the end, dismiss its result.
#[derive(Clone)]
pub struct FirmwareTask {
    inner: Arc<Inner>,
}

impl FirmwareTask {
    pub fn new(handle: Handle, target: Target) -> Self {
        let words = match target {
            Target::Device(_) => Words::Update,
            Target::NewBoard(_) => Words::Install,
        };
        Self {
            inner: Arc::new(Inner {
                handle,
                target,
                words,
                view: watch::channel(TaskView::default()).0,
                job: Mutex::new(None),
                context: Mutex::new(Context {
                    device: "the Device".into(),
                    version: String::new(),
                    switch_to: None,
                }),
            }),
        }
    }
    pub fn view(&self) -> TaskView {
        self.inner.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<TaskView> {
        self.inner.view.subscribe()
    }
    fn id(&self) -> &DeviceId {
        match &self.inner.target {
            Target::Device(id) | Target::NewBoard(id) => id,
        }
    }
    fn context(&self) -> Context {
        match &self.inner.target {
            Target::NewBoard(_) => Context {
                device: "the board".into(),
                version: String::new(),
                switch_to: None,
            },
            Target::Device(id) => {
                let device = self.inner.handle.devices().get(id);
                Context {
                    device: device
                        .as_ref()
                        .and_then(|d| d.name.clone())
                        .unwrap_or_else(|| "the Device".into()),
                    version: device
                        .and_then(|d| d.firmware)
                        .map(|f| f.version)
                        .unwrap_or_default(),
                    switch_to: None,
                }
            }
        }
    }
    fn show(&self, job: &Job) {
        if self.inner.job.lock().as_deref() != Some(job.id.as_str()) {
            return;
        }
        let context = self.inner.context.lock().clone();
        let (view, raw) = view_of(job, &context, self.inner.words);
        self.inner.view.send_modify(|task| {
            task.view = view;
            if task.log.last() != Some(&raw) {
                task.log.push(raw);
            }
        });
    }

    /// Follows `job` until it finishes. Subscribe before reading, so no update is lost.
    async fn follow(&self, job: Job, mut changes: tokio::sync::broadcast::Receiver<Change>) {
        *self.inner.job.lock() = Some(job.id.clone());
        self.inner.view.send_modify(|task| task.log.clear());
        let latest = self.inner.handle.jobs().get(&job.id).unwrap_or(job);
        self.show(&latest);
        if latest.state.finished() {
            return;
        }
        let id = latest.id;
        loop {
            match changes.recv().await {
                Ok(Change::JobUpdated(update)) if update.0.id == id => {
                    self.show(&update.0);
                    if update.0.state.finished() {
                        return;
                    }
                }
                Ok(Change::Stopping) | Err(RecvError::Closed) => return,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    changes = self.inner.handle.observe();
                    let Some(job) = self.inner.handle.jobs().get(&id) else {
                        return;
                    };
                    self.show(&job);
                    if job.state.finished() {
                        return;
                    }
                }
            }
            if self.inner.job.lock().as_deref() != Some(id.as_str()) {
                return;
            }
        }
    }

    /// Installs or updates to the Firmware this Companion carries, of the build the Device runs, and follows the Job
    /// to its end. Does nothing while a Job is in progress.
    pub async fn start(&self) {
        self.start_from(Source::Bundled, None).await;
    }

    /// Installs or updates from `source` and follows the Job to its end. `channel` names the build to install; naming
    /// the other build than the Device runs switches builds, which always asks for consent on this PC. Left out, it is
    /// the build the Device runs. Does nothing while a Job, or its file dialog, is in progress.
    pub async fn start_from(&self, source: Source, channel: Option<FirmwareChannel>) {
        let mut context = self.context();
        let running = self.running();
        context.switch_to = channel.filter(|channel| running.is_some_and(|r| r != *channel));
        let first = match source {
            Source::Bundled => TaskView {
                view: JobView::Running {
                    text: self.inner.words.progress(None),
                    pct: None,
                },
                log: Vec::new(),
            },
            Source::File => TaskView {
                view: JobView::Choosing {
                    text: CHOOSING.into(),
                },
                log: Vec::new(),
            },
        };
        // Claims the task, or leaves the one in progress alone.
        let mut before = None;
        let claimed = self.inner.view.send_if_modified(|task| {
            if task.view.in_progress() {
                return false;
            }
            before = Some(std::mem::replace(task, first));
            true
        });
        if !claimed {
            return;
        }
        *self.inner.context.lock() = context.clone();
        *self.inner.job.lock() = None;
        let firmware = self.inner.handle.firmware();
        let bundle = match source {
            Source::Bundled => {
                let wanted = channel.or(running).unwrap_or_default();
                if !firmware.carries(wanted) {
                    let details = serde_json::json!({ "channel": wanted });
                    self.fail(
                        NOT_BUNDLED,
                        Some(&details),
                        "No bundled Firmware of this build.",
                        &context,
                    );
                    return;
                }
                "bundled".to_owned()
            }
            Source::File => match firmware.pick(self.id().clone()).await {
                Ok(PickResult {
                    bundle: Some(bundle),
                }) => bundle.id,
                // The dialog closed without a file: back to what the task showed before.
                Ok(PickResult { bundle: None }) => {
                    self.inner
                        .view
                        .send_replace(before.take().unwrap_or_default());
                    return;
                }
                Err(error) => {
                    let body = error.body();
                    self.fail(&body.code, body.details.as_ref(), &body.message, &context);
                    return;
                }
            },
        };
        self.inner.view.send_replace(TaskView {
            view: JobView::Running {
                text: self.inner.words.progress(None),
                pct: None,
            },
            log: Vec::new(),
        });
        let changes = self.inner.handle.observe();
        let operation = match self.inner.target {
            Target::Device(_) => Operation::Update,
            Target::NewBoard(_) => Operation::Install,
        };
        let started = firmware.start(
            operation,
            self.id().clone(),
            FirmwareRequest { bundle, channel },
        );
        match started {
            Ok(job) => self.follow(job, changes).await,
            // Another start of the same target already runs: follow that one.
            Err(FirmwareError::Job(JobError::Busy(job))) => self.follow(*job, changes).await,
            Err(error) => {
                let body = error.body();
                self.fail(&body.code, body.details.as_ref(), &body.message, &context);
            }
        }
    }

    /// The build the target runs, when it reports one.
    fn running(&self) -> Option<FirmwareChannel> {
        match &self.inner.target {
            Target::NewBoard(_) => None,
            Target::Device(id) => self
                .inner
                .handle
                .devices()
                .get(id)
                .and_then(|d| d.firmware)
                .map(|f| f.channel),
        }
    }

    fn fail(&self, code: &str, details: Option<&Value>, message: &str, context: &Context) {
        let text = self.inner.words.failed(code, details, context);
        self.inner.view.send_replace(TaskView {
            view: JobView::Failed {
                text,
                code: code.to_owned(),
            },
            log: vec![format!("Failed: {message}")],
        });
    }

    /// Stops the followed Job while it can stop, such as while the consent dialog waits. That closes the dialog, and
    /// nothing changes on the Device.
    pub fn cancel(&self) {
        let job = self.inner.job.lock().clone();
        if let Some(job) = job {
            let _ = self.inner.handle.jobs().cancel(&job);
        }
    }

    /// Follows a Job that started elsewhere, such as one already running when the screen opened.
    pub async fn attach(&self, job: Job) {
        if self.inner.job.lock().as_deref() == Some(job.id.as_str()) {
            return;
        }
        *self.inner.context.lock() = self.context();
        let changes = self.inner.handle.observe();
        self.follow(job, changes).await;
    }

    /// Clears a finished result.
    pub fn dismiss(&self) {
        if !self.view().view.in_progress() {
            *self.inner.job.lock() = None;
            self.inner.view.send_replace(TaskView::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobScope;
    use serde_json::json;
    use std::cmp::Ordering;

    fn job(state: JobState, progress: Value) -> Job {
        Job {
            id: "j".into(),
            kind: "firmware.update".into(),
            scope: JobScope::Device("a".into()),
            state,
            progress,
            result: None,
            error: None,
            cancellable: false,
            created_ms: 0,
            finished_ms: None,
            rev: 1,
        }
    }
    fn context() -> Context {
        Context {
            device: "Desk".into(),
            version: "1.2.0".into(),
            switch_to: None,
        }
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert_eq!(compare_versions("1.10.0", "1.9.9"), Some(Ordering::Greater));
        assert_eq!(compare_versions("v1.2", "1.2.0"), Some(Ordering::Equal));
        assert_eq!(
            compare_versions("1.2.3-rc1", "1.2.3"),
            Some(Ordering::Equal)
        );
        assert_eq!(compare_versions("mock", "1.0.0"), None);
        assert_eq!(short_version("1.4.2"), "1.4");
        assert_eq!(short_version("mock"), "mock");
    }

    #[test]
    fn progress_reads_as_each_phase() {
        let words = |progress| {
            view_of(&job(JobState::Running, progress), &context(), Words::Update)
                .0
                .text()
                .to_owned()
        };
        assert_eq!(words(json!({"phase":"sending","pct":42})), "Updating… 42%");
        assert_eq!(words(json!({"phase":"rebooting"})), "Restarting…");
        assert_eq!(
            words(json!({"phase":"recovering","pct":7})),
            "Repairing… 7%"
        );
        assert_eq!(words(json!(null)), "Updating…");
        let (view, raw) = view_of(
            &job(JobState::Running, json!({"phase":"sending","pct":42})),
            &context(),
            Words::Install,
        );
        assert_eq!(view.text(), "Installing Firmware… 42%");
        assert_eq!(raw, "Sending 42%");
    }

    #[test]
    fn failures_say_what_still_runs() {
        let words = Words::Update;
        assert_eq!(
            words.failed("firmware.declined", None, &context()),
            "Declined on this PC. Desk still runs 1.2.0."
        );
        assert_eq!(
            words.failed(
                "firmware.failed",
                Some(&json!({"reason":"the port was unplugged"})),
                &context()
            ),
            "Desk was unplugged during the update. Plug it back in. If its screen stays blank, repair it from Troubleshooting."
        );
        assert_eq!(
            words.failed(
                "firmware.channel_mismatch",
                Some(&json!({"channel":"normal","bundle":"radio"})),
                &context()
            ),
            "That file is the radio build, and Desk runs the normal build. Nothing changed."
        );
        assert_eq!(
            Words::Install.failed("firmware.declined", None, &context()),
            DECLINED
        );
        assert_eq!(
            Words::Install.failed("firmware.failed", None, &context()),
            INSTALL_FAILED
        );
    }

    #[test]
    fn a_board_card_follows_its_install() {
        assert_eq!(board_step(&JobView::Idle), BoardStep::Detected);
        let failed = |code: &str| JobView::Failed {
            text: String::new(),
            code: code.into(),
        };
        assert_eq!(
            board_step(&failed("firmware.declined")),
            BoardStep::Declined
        );
        assert_eq!(board_step(&failed("firmware.failed")), BoardStep::Failed);
        let mut done = job(JobState::Succeeded, json!(null));
        done.result = Some(json!({"message":"Firmware installed","version":"1.3.0"}));
        let (view, _) = view_of(&done, &context(), Words::Install);
        assert_eq!(board_step(&view), BoardStep::Done);
        assert_eq!(view.text(), "Firmware installed");
        let (view, _) = view_of(&done, &context(), Words::Update);
        assert_eq!(view.text(), "Done. Firmware installed.");
    }
}
