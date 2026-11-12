//! Firmware behaviour through the native UI seam. No listener, session or HTTP adapter.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use td_companion::{
    companion::Change,
    devices::{BoardKind, DeviceId, FirmwareChannel, LinkState, SetupState},
    features::firmware::{
        Confirmation, ConsentAnswer, FirmwareError, FirmwareRequest, FirmwareResult, Operation,
        Prompter,
    },
    jobs::{Job, JobError, JobScope, JobState},
    testing::InProcessCompanion,
};
use td_link::bundle::Bundle;

const NORMAL: &[u8] = include_bytes!("../../td-link/tests/fixtures/touchdeck-test.tdfw");
const RADIO: &[u8] = include_bytes!("../../td-link/tests/fixtures/touchdeck-test-radio.tdfw");
const V1: &[u8] = include_bytes!("fixtures/firmware-v1.tdfw");

struct Prompt {
    bytes: &'static [u8],
    seen: Mutex<Vec<Confirmation>>,
    answer: Mutex<Option<bool>>,
    closed: AtomicBool,
    accept_on_close: bool,
}
impl Prompt {
    fn new(bytes: &'static [u8]) -> Self {
        Self {
            bytes,
            seen: Default::default(),
            answer: Default::default(),
            closed: Default::default(),
            accept_on_close: false,
        }
    }
    fn answer(&self, answer: bool) {
        *self.answer.lock().unwrap() = Some(answer);
    }
}
impl Prompter for Prompt {
    fn pick(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(self.bytes.to_vec()))
    }
    fn confirm(&self, confirmation: &Confirmation, closed: &(dyn Fn() -> bool + Sync)) -> bool {
        self.seen.lock().unwrap().push(confirmation.clone());
        let started = Instant::now();
        loop {
            if closed() {
                self.closed.store(true, Ordering::SeqCst);
                return self.accept_on_close;
            }
            if let Some(answer) = self.answer.lock().unwrap().take() {
                return answer;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "consent did not end"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
fn bundled(channel: Option<FirmwareChannel>) -> FirmwareRequest {
    FirmwareRequest {
        bundle: "bundled".into(),
        channel,
    }
}
fn selected(id: &str) -> FirmwareRequest {
    FirmwareRequest {
        bundle: id.into(),
        channel: None,
    }
}
async fn wait(td: &InProcessCompanion, job: &Job) -> Job {
    td.until(|td| td.handle.jobs().get(&job.id).unwrap().state.finished())
        .await;
    td.handle.jobs().get(&job.id).unwrap()
}
async fn prompted(td: &InProcessCompanion, prompt: &Prompt, count: usize) {
    td.until(|_| prompt.seen.lock().unwrap().len() == count)
        .await;
}
async fn ready(td: &InProcessCompanion, id: &DeviceId, channel: FirmwareChannel) {
    td.until(|td| {
        td.device(id).is_some_and(|d| {
            matches!(d.link, LinkState::Connected { .. })
                && d.firmware.is_some_and(|f| f.channel == channel)
        })
    })
    .await;
}
fn no_writes(td: &InProcessCompanion, serial: &str, reboots: u32) {
    let deck = td.deck(serial);
    assert_eq!(deck.reboots(), reboots);
    assert!(!deck.received().iter().any(|v| v["m"] == "update.begin"));
}

#[tokio::test]
async fn selection_is_target_bound_consumed_on_start_and_decline_releases_the_hold() {
    let td = InProcessCompanion::start().await;
    let a = td.plug_device("mocka").await;
    let b = td.plug_device("mockb").await;
    let prompt = Arc::new(Prompt::new(NORMAL));
    td.handle.firmware().set_prompter(prompt.clone());
    let info = td
        .handle
        .firmware()
        .pick(a.clone())
        .await
        .unwrap()
        .bundle
        .unwrap();
    assert!(matches!(
        td.handle
            .firmware()
            .start(Operation::Update, b.clone(), selected(&info.id)),
        Err(FirmwareError::Missing)
    ));
    let job = td
        .handle
        .firmware()
        .start(Operation::Repair, a.clone(), selected(&info.id))
        .unwrap();
    prompted(&td, &prompt, 1).await;
    assert_eq!(
        td.handle.jobs().get(&job.id).unwrap().state,
        JobState::WaitingForConfirmation
    );
    assert!(
        matches!(td.device(&a).unwrap().link, LinkState::Updating { job: ref id } if id == &job.id)
    );
    assert!(matches!(
        td.device(&b).unwrap().link,
        LinkState::Connected { .. }
    ));
    let busy = td
        .handle
        .firmware()
        .start(Operation::Update, a.clone(), bundled(None));
    assert!(matches!(busy, Err(FirmwareError::Job(JobError::Busy(held))) if held.id == job.id));
    assert!(matches!(
        td.handle
            .firmware()
            .start(Operation::Repair, a.clone(), selected(&info.id)),
        Err(FirmwareError::Missing)
    ));
    {
        let seen = prompt.seen.lock().unwrap();
        assert_eq!(seen[0].target, a.as_str());
        assert_eq!(seen[0].version, info.version);
        assert_eq!(seen[0].build, info.build);
        assert_eq!(seen[0].current, "mock");
        assert!(!seen[0].port.is_empty());
    }
    prompt.answer(false);
    let done = wait(&td, &job).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "firmware.declined");
    no_writes(&td, "mocka", 0);
    assert!(td.handle.jobs().holding(&job.scope).is_none());
    ready(&td, &a, FirmwareChannel::Normal).await;
}

#[tokio::test]
async fn selection_expires_at_ten_minutes_without_starting_a_job() {
    let td = InProcessCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let prompt = Arc::new(Prompt::new(NORMAL));
    td.handle.firmware().set_prompter(prompt.clone());
    let info = td
        .handle
        .firmware()
        .pick(id.clone())
        .await
        .unwrap()
        .bundle
        .unwrap();
    td.advance_clock(Duration::from_secs(600));
    assert!(matches!(
        td.handle
            .firmware()
            .start(Operation::Update, id, selected(&info.id)),
        Err(FirmwareError::Missing)
    ));
    assert!(td.handle.jobs().list().is_empty());
    assert!(prompt.seen.lock().unwrap().is_empty());
    no_writes(&td, "mocka", 0);
}

#[tokio::test]
async fn cancellation_and_consent_expiry_reject_even_a_late_acceptance() {
    for cancel in [false, true] {
        let td = InProcessCompanion::start().await;
        let id = td.plug_device("mocka").await;
        let prompt = Arc::new(Prompt {
            accept_on_close: true,
            ..Prompt::new(NORMAL)
        });
        td.handle.firmware().set_prompter(prompt.clone());
        let job = td
            .handle
            .firmware()
            .start(Operation::Repair, id.clone(), bundled(None))
            .unwrap();
        prompted(&td, &prompt, 1).await;
        if cancel {
            td.handle.jobs().cancel(&job.id).unwrap();
        } else {
            td.advance_clock(Duration::from_secs(299));
            assert_eq!(
                td.handle.jobs().get(&job.id).unwrap().state,
                JobState::WaitingForConfirmation
            );
            td.advance_clock(Duration::from_secs(2));
        }
        let done = wait(&td, &job).await;
        assert!(prompt.closed.load(Ordering::SeqCst));
        assert_eq!(
            done.state,
            if cancel {
                JobState::Cancelled
            } else {
                JobState::Failed
            }
        );
        assert_eq!(
            done.error.unwrap().code,
            if cancel {
                "job.cancelled"
            } else {
                "firmware.declined"
            }
        );
        no_writes(&td, "mocka", 0);
        assert!(td.handle.jobs().holding(&job.scope).is_none());
        ready(&td, &id, FirmwareChannel::Normal).await;
    }
}

#[tokio::test]
async fn downgrade_consent_binds_the_bundle_and_suppresses_input_from_every_device() {
    let td = InProcessCompanion::start().await;
    let id = td.plug_device("mocka").await;
    td.plug_device("mockb").await;
    let mut current = Bundle::parse(NORMAL).unwrap();
    current.manifest.version = "2.0.0".into();
    td.deck("mocka").flash_bundle(&current);
    td.handle.devices().restart(&id).await.unwrap().unwrap();
    td.until(|td| {
        td.device(&id).is_some_and(|d| {
            matches!(d.link, LinkState::Connected { .. })
                && d.firmware.is_some_and(|f| f.version == "2.0.0")
        })
    })
    .await;
    let reboots = td.deck("mocka").reboots();
    let prompt = Arc::new(Prompt::new(V1));
    td.handle.firmware().set_prompter(prompt.clone());
    let info = td
        .handle
        .firmware()
        .pick(id.clone())
        .await
        .unwrap()
        .bundle
        .unwrap();
    let job = td
        .handle
        .firmware()
        .start(Operation::Update, id.clone(), selected(&info.id))
        .unwrap();
    prompted(&td, &prompt, 1).await;
    {
        let seen = prompt.seen.lock().unwrap();
        assert_eq!(seen[0].question, "Downgrade this Device's Firmware?");
        assert_eq!(seen[0].current, "2.0.0");
        assert_eq!(seen[0].version, "1.0.0");
        assert_eq!(seen[0].build, info.build);
        assert_eq!(seen[0].target, id.as_str());
    }
    let mut changes = td.handle.observe();
    for serial in ["mocka", "mockb"] {
        td.deck(serial)
            .emit_key(&serde_json::json!({"type":"kbd", "mods":0, "keys":[4]}));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut suppressed = std::collections::BTreeSet::new();
        while suppressed.len() < 2 {
            if let Change::DeviceKeyMirror(event) = changes.recv().await.unwrap() {
                assert_eq!(event.outcome, "suppressed");
                suppressed.insert(event.device);
            }
        }
    })
    .await
    .unwrap();
    assert!(td.input_log().is_empty());
    no_writes(&td, "mocka", reboots);
    prompt.answer(false);
    assert_eq!(
        wait(&td, &job).await.error.unwrap().code,
        "firmware.declined"
    );
    ready(&td, &id, FirmwareChannel::Normal).await;
    td.deck("mockb")
        .emit_key(&serde_json::json!({"type":"kbd", "mods":0, "keys":[5]}));
    td.deck("mockb")
        .emit_key(&serde_json::json!({"type":"kbd", "mods":0, "keys":[]}));
    td.until(|td| td.input_log() == ["down B", "up B"]).await;
}

#[tokio::test]
async fn mismatched_files_are_refused_and_switching_builds_requires_exact_consent() {
    let td = InProcessCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let builds = td.handle.firmware().builds().unwrap();
    let radio_build = builds.radio.unwrap().build;
    let prompt = Arc::new(Prompt::new(RADIO));
    td.handle.firmware().set_prompter(prompt.clone());
    let info = td
        .handle
        .firmware()
        .pick(id.clone())
        .await
        .unwrap()
        .bundle
        .unwrap();
    assert!(matches!(
        td.handle
            .firmware()
            .start(Operation::Update, id.clone(), selected(&info.id)),
        Err(FirmwareError::ChannelMismatch {
            channel: FirmwareChannel::Normal,
            bundle: FirmwareChannel::Radio
        })
    ));
    assert!(td.handle.jobs().list().is_empty());
    assert!(prompt.seen.lock().unwrap().is_empty());
    // A rejected mismatch does not consume the file. An explicit switch can still use it.
    let job = td
        .handle
        .firmware()
        .start(
            Operation::Update,
            id.clone(),
            FirmwareRequest {
                bundle: info.id,
                channel: Some(FirmwareChannel::Radio),
            },
        )
        .unwrap();
    prompted(&td, &prompt, 1).await;
    prompt.answer(false);
    assert_eq!(
        wait(&td, &job).await.error.unwrap().code,
        "firmware.declined"
    );
    no_writes(&td, "mocka", 0);
    ready(&td, &id, FirmwareChannel::Normal).await;
    for (index, channel, build) in [
        (2, FirmwareChannel::Radio, radio_build),
        (3, FirmwareChannel::Normal, builds.build),
    ] {
        let job = td
            .handle
            .firmware()
            .start(Operation::Update, id.clone(), bundled(Some(channel)))
            .unwrap();
        prompted(&td, &prompt, index).await;
        {
            let seen = prompt.seen.lock().unwrap();
            let c = seen.last().unwrap();
            assert!(
                c.question
                    .starts_with(if channel == FirmwareChannel::Radio {
                        "Switch this Device to the radio build?"
                    } else {
                        "Switch this Device back to the normal build?"
                    })
            );
            assert_eq!(c.channel, channel);
            assert_eq!(c.build, build);
        }
        prompt.answer(true);
        let done = wait(&td, &job).await;
        assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
        ready(&td, &id, channel).await;
        assert_eq!(td.device(&id).unwrap().firmware.unwrap().build, build);
    }
}

#[tokio::test]
async fn same_build_updates_need_no_prompt_and_report_progress_and_rollback() {
    for rollback in [false, true] {
        let td = InProcessCompanion::start().await;
        let id = td.plug_device("mocka").await;
        td.deck("mocka").flash_bundle(
            &Bundle::parse(include_bytes!(
                "../../td-link/tests/fixtures/touchdeck-test-b.tdfw"
            ))
            .unwrap(),
        );
        td.handle.devices().restart(&id).await.unwrap().unwrap();
        td.until(|td| {
            td.device(&id).is_some_and(|d| {
                matches!(d.link, LinkState::Connected { .. })
                    && d.firmware.is_some_and(|f| f.version == "test-only")
            })
        })
        .await;
        let prompt = Arc::new(Prompt::new(NORMAL));
        td.handle.firmware().set_prompter(prompt.clone());
        if rollback {
            td.deck("mocka").update_mode("rollback");
        }
        let mut changes = td.handle.observe();
        let job = td
            .handle
            .firmware()
            .start(Operation::Update, id, bundled(None))
            .unwrap();
        let mut phases = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Change::JobUpdated(event) = changes.recv().await.unwrap() {
                    let changed = event.0;
                    if changed.id != job.id {
                        continue;
                    }
                    if let Some(phase) = changed.progress["phase"].as_str() {
                        if phases.last().map(String::as_str) != Some(phase) {
                            phases.push(phase.to_owned());
                        }
                        if phase == "sending" {
                            assert!(!changed.cancellable);
                        }
                    }
                    if changed.state.finished() {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        let done = wait(&td, &job).await;
        assert!(prompt.seen.lock().unwrap().is_empty());
        assert_eq!(phases, ["checking", "sending", "rebooting", "confirming"]);
        if rollback {
            assert_eq!(done.state, JobState::Failed);
            assert_eq!(done.error.unwrap().code, "firmware.rolled_back");
        } else {
            assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
            let result: FirmwareResult = serde_json::from_value(done.result.unwrap()).unwrap();
            assert_eq!(result.version, "test-only");
        }
    }
}

#[tokio::test]
async fn disconnects_before_start_and_during_consent_fail_without_writing() {
    let td = InProcessCompanion::start().await;
    let id = td.plug_device("mocka").await;
    let deck = td.deck("mocka");
    let prompt = Arc::new(Prompt::new(NORMAL));
    td.handle.firmware().set_prompter(prompt.clone());
    let job = td
        .handle
        .firmware()
        .start(Operation::Update, id.clone(), bundled(None))
        .unwrap();
    prompted(&td, &prompt, 1).await;
    // Firmware holds discovery; change the physical world without waiting for a rescan.
    td.ports.unplug("mocka");
    prompt.answer(true);
    let done = wait(&td, &job).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "firmware.failed");
    assert_eq!(deck.reboots(), 0);
    assert!(!deck.received().iter().any(|v| v["m"] == "update.begin"));
    assert!(td.handle.jobs().holding(&job.scope).is_none());
    td.unplug("mocka").await;
    assert!(matches!(
        td.handle
            .firmware()
            .start(Operation::Update, id, bundled(None)),
        Err(FirmwareError::Device(_))
    ));
}

#[tokio::test]
async fn disconnect_between_job_reservation_and_device_hold_releases_occupancy() {
    let td = InProcessCompanion::start().await;
    let id = td.plug_device("mocka").await;
    td.handle.jobs().hold();
    let job = td
        .handle
        .firmware()
        .start(Operation::Update, id.clone(), bundled(None))
        .unwrap();
    td.unplug("mocka").await;
    td.handle.jobs().release();
    let done = wait(&td, &job).await;
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.unwrap().code, "firmware.failed");
    assert!(td.handle.jobs().holding(&job.scope).is_none());
}

#[tokio::test]
async fn new_board_install_uses_scripted_consent_and_reports_recovery_progress() {
    let td = InProcessCompanion::start().await;
    let board = td.plug_board("newboard", BoardKind::OtherFirmware).await;
    td.handle
        .firmware()
        .script(vec![ConsentAnswer::Decline, ConsentAnswer::Accept]);
    let job = td
        .handle
        .firmware()
        .start(Operation::Install, board.id.clone(), bundled(None))
        .unwrap();
    assert_eq!(job.scope, JobScope::NewBoard(board.id.to_string()));
    assert_eq!(
        wait(&td, &job).await.error.unwrap().code,
        "firmware.declined"
    );
    no_writes(&td, "newboard", 0);
    assert!(td.device(&board.id).is_none());
    let job = td
        .handle
        .firmware()
        .start(
            Operation::Install,
            board.id.clone(),
            bundled(Some(FirmwareChannel::Radio)),
        )
        .unwrap();
    let done = wait(&td, &job).await;
    assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
    assert_eq!(done.progress["phase"], "recovering");
    assert_eq!(done.progress["pct"], 100);
    let bytes: usize = Bundle::parse(RADIO)
        .unwrap()
        .images
        .iter()
        .map(|image| image.data.len())
        .sum();
    assert_eq!(done.progress["bytes"], bytes);
    ready(&td, &board.id, FirmwareChannel::Radio).await;
    assert_eq!(td.device(&board.id).unwrap().setup, SetupState::NeedsSetup);
}
