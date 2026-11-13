use super::{EditorError, Session, SessionError};
use crate::{
    companion::Handle,
    editor::PushPlan,
    jobs::{Job, JobState},
    layouts::{IfMatch, LayoutError, LayoutRead},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use td_core::editing::Capabilities;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    None,
    Saved,
    Unsent,
    Unconfirmed,
    Sending,
}
/// Owns immutable transfer work; it never borrows the Session or the UI thread.
pub struct PendingSend {
    owner: std::sync::Arc<()>,
    task: tokio::task::JoinHandle<SendReceipt>,
    observation_clock: Arc<AtomicU64>,
    plan: PushPlan,
    serial: u64,
}
/// A completion may be applied only to its originating Session's active send.
pub struct SendReceipt {
    observation: u64,
    owner: std::sync::Arc<()>,
    serial: u64,
    plan: PushPlan,
    result: Result<LayoutRead, SessionError>,
    conflict: Option<Option<LayoutRead>>,
    job: Option<Job>,
}
impl PendingSend {
    pub async fn wait(self) -> SendReceipt {
        match self.task.await {
            Ok(receipt) => receipt,
            Err(error) => SendReceipt {
                observation: self.observation_clock.fetch_add(1, Ordering::SeqCst) + 1,
                owner: self.owner,
                serial: self.serial,
                plan: self.plan,
                result: Err(SessionError::Send(error.to_string())),
                conflict: None,
                job: None,
            },
        }
    }
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}
impl Session {
    pub(super) fn next_observation(&self) -> u64 {
        self.observation_clock.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub fn save_state(&self) -> SaveState {
        if self.sending.is_some() {
            SaveState::Sending
        } else {
            match self.draft() {
                None => SaveState::None,
                Some(d) if d.is_unconfirmed() => SaveState::Unconfirmed,
                Some(d) if d.is_sent() => SaveState::Saved,
                Some(_) => SaveState::Unsent,
            }
        }
    }
    pub fn push_progress(&self) -> Option<crate::layouts::PushProgress> {
        let job = self
            .last_job
            .as_ref()
            .filter(|j| j.kind == "layouts.push")?;
        serde_json::from_value(job.progress.clone()).ok()
    }
    pub fn push_report(&self) -> Option<crate::layouts::PushReport> {
        let job = self
            .last_job
            .as_ref()
            .filter(|j| j.kind == "layouts.push" && j.state == JobState::Succeeded)?;
        serde_json::from_value(job.result.clone()?).ok()
    }
    /// Copy for a send refusal; structured details remain available for diagnostics.
    pub fn send_problem(&self, error: &SessionError) -> super::Notice {
        let device = self.device_name();
        let refusal = error.refusal();
        if let Some(refusal) = &refusal
            && let Some(problem) = self.gif_problem(refusal, true)
        {
            return problem;
        }
        let code = refusal
            .as_ref()
            .map(|e| e.code.as_str())
            .unwrap_or_default();
        let text = match code {
            "device.busy" => format!("{device} is busy. Wait a moment, then retry."),
            "job.cancelled" => format!("Sending stopped. {device} keeps its Layout."),
            "layout.invalid" => {
                let issue = refusal
                    .as_ref()
                    .and_then(|e| e.details.as_ref())
                    .and_then(|d| d.get("issues"))
                    .and_then(|i| i.get(0));
                match issue.and_then(|i| i.get("rule")).and_then(|v| v.as_str()) {
                    Some("label.too-long") => "Tile labels can be up to 16 characters.".into(),
                    Some("label.blank") => "Give the Tile a name.".into(),
                    _ => "That change breaks a Layout rule.".into(),
                }
            }
            _ => format!("Couldn't send to {device}. Check that it's plugged in, then retry."),
        };
        let detail = refusal
            .map(|e| {
                format!(
                    "{}: {}{}",
                    e.code,
                    e.message,
                    e.details.map(|d| format!(" {d}")).unwrap_or_default()
                )
            })
            .unwrap_or_else(|| error.to_string());
        super::Notice { text, detail }
    }
    /// Start a push snapshot. Edits and change processing remain available while it runs.
    pub fn begin_send(
        &mut self,
        capabilities: Capabilities<'_>,
    ) -> Result<PendingSend, EditorError> {
        if self.sending.is_some() {
            return Err(EditorError::Sending);
        }
        self.refresh_assets()?;
        let plan = match self.model.prepare_push(self.orientation, capabilities) {
            Ok(plan) => plan,
            Err(error) => {
                self.select_first_problem();
                return Err(error);
            }
        };
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| EditorError::RuntimeUnavailable)?;
        // Persist before starting owned work: losing its reply (or the process) cannot
        // turn a value-equal old draft into evidence that the write was rejected.
        self.model.mark_unconfirmed(self.orientation)?;
        self.model.persist()?;
        self.send_serial += 1;
        let serial = self.send_serial;
        self.sending = Some(serial);
        let (handle, copy, owner) = (self.handle.clone(), plan.clone(), self.send_owner.clone());
        let clock = self.observation_clock.clone();
        Ok(PendingSend {
            owner: self.send_owner.clone(),
            observation_clock: clock.clone(),
            task: runtime.spawn(async move { execute(handle, copy, serial, owner, clock).await }),
            plan,
            serial,
        })
    }
    pub fn finish_send(&mut self, receipt: SendReceipt) -> Result<(), SessionError> {
        if !std::sync::Arc::ptr_eq(&receipt.owner, &self.send_owner)
            || receipt.plan.device != self.device
            || self.sending != Some(receipt.serial)
        {
            return Err(EditorError::StalePush.into());
        }
        self.sending = None;
        let orientation = receipt.plan.layout().orientation;
        let observed = &mut self.observed[crate::editor::index(orientation)];
        // Equal Layout values or revisions do not make an old observation current.
        // Gate every result, including failures, before any draft or Job mutation.
        if receipt.observation <= *observed {
            return Err(EditorError::StalePush.into());
        }
        *observed = receipt.observation;
        self.last_job = receipt.job;
        match receipt.result {
            Ok(read) => {
                if let Err(error) =
                    self.model
                        .finish_push(receipt.plan, read.layout.clone(), read.revision.clone())
                {
                    if matches!(error, EditorError::PushMismatch) {
                        self.model.receive(read.layout, read.revision)?;
                        self.model.persist()?;
                    }
                    return Err(error.into());
                }
            }
            Err(error) => {
                self.model.mark_unconfirmed(orientation)?;
                if matches!(
                    &error,
                    SessionError::Layout(LayoutError::NotFound | LayoutError::StoredInvalid(_))
                ) {
                    self.model.receive_missing(orientation)?;
                    self.model.persist()?;
                }
                if let Some(remote) = receipt.conflict {
                    if self.model.draft(orientation).is_none_or(|draft| {
                        draft.known != receipt.plan.baseline || draft.conflict.is_some()
                    }) {
                        return Err(EditorError::StalePush.into());
                    }
                    match remote {
                        Some(read) => self.model.receive(read.layout, read.revision)?,
                        None => self.model.receive_missing(orientation)?,
                    }
                    self.model.persist()?;
                }
                self.model.persist()?;
                return Err(error);
            }
        }
        self.refresh_assets()?;
        self.sync_gifs()?;
        self.model.persist()?;
        Ok(())
    }
}
async fn execute(
    handle: Handle,
    plan: PushPlan,
    serial: u64,
    owner: std::sync::Arc<()>,
    observation_clock: Arc<AtomicU64>,
) -> SendReceipt {
    let mut changes = handle.observe();
    let operations = handle.layout_operations();
    let device = plan.device.clone();
    let orientation = plan.layout().orientation;
    let mut last = None;
    // Number each read when it is issued, and a failure before any read when it is known.
    // The readback is issued after the write finished, so a Load issued before it may
    // predate the write and must rank lower; a Load issued after it sees the write too.
    let next = || observation_clock.fetch_add(1, Ordering::SeqCst) + 1;
    let mut read_at = None;
    let result: Result<LayoutRead, SessionError> = async {
        let job = operations
            .push(
                device.clone(),
                orientation,
                plan.layout().to_canonical_json(),
                plan.expected_revision().map(|r| IfMatch(vec![r.into()])),
            )
            .await?;
        loop {
            let latest = handle
                .jobs()
                .get(&job.id)
                .ok_or_else(|| SessionError::Send("Job expired".into()))?;
            last = Some(latest.clone());
            if latest.state.finished() {
                if latest.state != JobState::Succeeded {
                    return Err(latest.error.map(SessionError::Job).unwrap_or_else(|| {
                        SessionError::Send("Sending stopped. The Device keeps its Layout.".into())
                    }));
                }
                break;
            }
            match changes.recv().await {
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return Err(SessionError::Send("Companion stopped".into())),
            }
        }
        #[cfg(any(test, feature = "test-hooks"))]
        {
            let checkpoints = handle.checkpoints.clone();
            tokio::task::spawn_blocking(move || {
                checkpoints.pass(crate::checkpoints::Checkpoint::BeforeEditorReadback)
            })
            .await
            .map_err(|e| SessionError::Send(e.to_string()))?
            .map_err(|_| LayoutError::Device(crate::link::DeviceLinkError::Timeout))?;
        }
        read_at = Some(next());
        Ok(operations.read(device.clone(), orientation).await?)
    }
    .await;
    let conflict = if result
        .as_ref()
        .err()
        .and_then(|e| e.refusal())
        .is_some_and(|e| e.code == "layout.changed")
    {
        read_at = Some(next());
        match operations.read(device, orientation).await {
            Ok(read) => Some(Some(read)),
            Err(LayoutError::NotFound) => Some(None),
            Err(error) => {
                return SendReceipt {
                    observation: read_at.unwrap_or_else(next),
                    owner,
                    serial,
                    plan,
                    result: Err(error.into()),
                    conflict: None,
                    job: last,
                };
            }
        }
    } else {
        None
    };
    SendReceipt {
        observation: read_at.unwrap_or_else(next),
        owner,
        serial,
        plan,
        result,
        conflict,
        job: last,
    }
}
