//! A Firmware Job as a screen follows it: one install on a New board or one update of a Device,
//! with the words for each step. Ported from the 1.x app's `lib/device/firmware.ts` and
//! `new-board.ts`, so setup and Settings word progress and failures the same way.
use super::{FirmwareBuild, FirmwareError, FirmwareRequest, Operation};
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
        matches!(self, Self::Waiting { .. } | Self::Running { .. })
    }
    pub fn text(&self) -> &str {
        match self {
            Self::Idle => "",
            Self::Waiting { text }
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

const WAITING: &str = "Confirm on this PC to continue.";
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
}

fn reason(details: Option<&Value>) -> &str {
    details
        .and_then(|d| d.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("")
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
    fn done(self, message: &str, version: &str) -> String {
        match self {
            Words::Install => "Firmware installed".into(),
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
            return if code == "firmware.declined" {
                DECLINED.into()
            } else {
                INSTALL_FAILED.into()
            };
        }
        let Context { device, version } = context;
        match code {
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
                    text: words.done(&message, &version),
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
        JobView::Waiting { .. } => BoardStep::Waiting,
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

    /// Follows `job` until it finishes. `changes` must be subscribed before `job` was read, so it
    /// holds every update after `job`. Each update shows in order, even when the Job finished
    /// before this runs: jumping to the Job's latest copy would drop the steps between from Details.
    async fn follow(&self, job: Job, mut changes: tokio::sync::broadcast::Receiver<Change>) {
        *self.inner.job.lock() = Some(job.id.clone());
        self.inner.view.send_modify(|task| task.log.clear());
        self.show(&job);
        if job.state.finished() {
            return;
        }
        let Job { id, mut rev, .. } = job;
        // Shows `job` unless it's older than what shows now. Returns whether it finished.
        let mut next = |job: &Job| {
            if job.rev > rev {
                rev = job.rev;
                self.show(job);
            }
            job.state.finished()
        };
        loop {
            match changes.recv().await {
                Ok(Change::JobUpdated(update)) if update.0.id == id => {
                    if next(&update.0) {
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
                    if next(&job) {
                        return;
                    }
                }
            }
            if self.inner.job.lock().as_deref() != Some(id.as_str()) {
                return;
            }
        }
    }

    /// Installs or updates to the Firmware this Companion carries, and follows the Job to its
    /// end. Does nothing while a Job is in progress.
    pub async fn start(&self) {
        if self.view().view.in_progress() {
            return;
        }
        let context = self.context();
        *self.inner.context.lock() = context.clone();
        *self.inner.job.lock() = None;
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
        let started = self.inner.handle.firmware().start(
            operation,
            self.id().clone(),
            FirmwareRequest {
                bundle: "bundled".into(),
                channel: None,
            },
        );
        match started {
            Ok(job) => self.follow(job, changes).await,
            // Another start of the same target already runs: follow that one.
            Err(FirmwareError::Job(JobError::Busy(job))) => self.follow(*job, changes).await,
            Err(error) => {
                let body = error.body();
                let text = self
                    .inner
                    .words
                    .failed(&body.code, body.details.as_ref(), &context);
                self.inner.view.send_replace(TaskView {
                    view: JobView::Failed {
                        text,
                        code: body.code,
                    },
                    log: vec![format!("Failed: {}", body.message)],
                });
            }
        }
    }

    /// Follows a Job that started elsewhere, such as one already running when the screen opened.
    pub async fn attach(&self, job: Job) {
        if self.inner.job.lock().as_deref() == Some(job.id.as_str()) {
            return;
        }
        *self.inner.context.lock() = self.context();
        let changes = self.inner.handle.observe();
        // `job` was read before this subscribed, so read it again; a finished Job may be gone.
        let job = self.inner.handle.jobs().get(&job.id).unwrap_or(job);
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

    /// Waits on `changes` until the Job `id` finishes.
    async fn finished(changes: &mut tokio::sync::broadcast::Receiver<Change>, id: &str) {
        loop {
            if let Change::JobUpdated(update) = changes.recv().await.unwrap()
                && update.0.id == id
                && update.0.state.finished()
            {
                return;
            }
        }
    }

    // A scripted Accept lets an install run through every step before `start` reads it: Details
    // still lists each step, in order, once.
    #[tokio::test]
    async fn details_keep_every_step_of_a_job_that_finished_before_it_was_read() {
        use crate::devices::BoardKind;
        use crate::testing::InProcessCompanion;
        use std::time::Duration;

        let mut td = InProcessCompanion::start().await;
        let board = td.plug_board("mock0009", BoardKind::Silent).await;
        let task = FirmwareTask::new(td.handle.clone(), Target::NewBoard(board.id.clone()));
        td.handle
            .firmware()
            .script(vec![super::super::ConsentAnswer::Accept]);
        let changes = td.handle.observe();
        let mut ends = td.handle.observe();
        let job = td
            .handle
            .firmware()
            .start(
                Operation::Install,
                board.id.clone(),
                FirmwareRequest {
                    bundle: "bundled".into(),
                    channel: None,
                },
            )
            .unwrap();
        // The order the flaky test hit by chance: the Job ends, then `start` follows it.
        finished(&mut ends, &job.id).await;
        tokio::time::timeout(Duration::from_secs(10), task.follow(job.clone(), changes))
            .await
            .expect("followed to the end");
        let view = task.view();
        assert_eq!(view.view.text(), "Firmware installed");
        let at = |line: &str| view.log.iter().position(|l| l == line);
        let waiting = at("Waiting for confirmation").unwrap_or_else(|| panic!("{:?}", view.log));
        assert!(
            at("Queued").is_some_and(|queued| queued < waiting),
            "{:?}",
            view.log
        );
        assert_eq!(
            view.log.last().map(String::as_str),
            Some("Firmware installed")
        );
        assert_eq!(
            view.log
                .iter()
                .filter(|l| *l == "Waiting for confirmation")
                .count(),
            1,
            "{:?}",
            view.log
        );

        // A screen that opens later attaches with an old copy of the Job: it shows the end and stops.
        let late = FirmwareTask::new(td.handle.clone(), Target::NewBoard(board.id.clone()));
        tokio::time::timeout(Duration::from_secs(10), late.attach(job))
            .await
            .expect("attach stops at the end");
        assert_eq!(late.view().view.text(), "Firmware installed");
        td.shutdown().await;
    }
}
