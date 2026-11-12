mod prompt;
use crate::{
    api::{Event, Events},
    clock::Clock,
    devices::{DeviceId, DeviceManager},
    link::Links,
};
use parking_lot::Mutex;
#[cfg(any(test, feature = "test-hooks"))]
pub use prompt::FakePrompter;
pub use prompt::{ApprovalPrompter, NativePrompter, NoopNotifier, TrayNotifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use td_core::layout::{Layout, Page, Tile};
use td_host::{actions::Plan, approval::CommandSpec};
use ts_rs::TS;

const TAP_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ActionSpec {
    pub kind: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub shell: bool,
}
impl ActionSpec {
    pub fn hash(&self) -> String {
        Sha256::digest(serde_json::to_vec(self).expect("action serializes"))
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
    fn command(&self) -> CommandSpec {
        CommandSpec {
            program: self.program.clone(),
            args: self.args.clone(),
            cwd: self.cwd.clone(),
            shell: self.shell,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ApprovalRequest {
    pub id: String,
    pub device: DeviceId,
    pub page: String,
    pub tile: String,
    pub hash: String,
    #[serde(flatten)]
    pub spec: ActionSpec,
    pub created_ms: u64,
}
impl Event for ApprovalRequest {
    const NAME: &'static str = "approval.requested";
    fn device(&self) -> Option<&str> {
        Some(&self.device)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    Once,
    Always,
    Deny,
    Expired,
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct ApprovalResolved {
    pub id: String,
    pub answer: Answer,
}
impl Event for ApprovalResolved {
    const NAME: &'static str = "approval.resolved";
}
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct AllowedAction {
    pub hash: String,
    #[serde(flatten)]
    pub spec: ActionSpec,
    pub granted_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ActionLog {
    pub id: String,
    pub device: DeviceId,
    pub at_ms: u64,
    pub page: String,
    pub tile: String,
    pub kind: String,
    pub status: String,
    pub detail: String,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
}
impl Event for ActionLog {
    const NAME: &'static str = "action.logged";
    fn device(&self) -> Option<&str> {
        Some(&self.device)
    }
}
/// Where "Always allow" answers are kept. Blocking: call it off the async runtime.
pub trait GrantStore: Send + Sync {
    /// The saved grants; none when nothing was saved yet.
    fn load(&self) -> std::io::Result<Vec<AllowedAction>>;
    /// Replaces the saved grants. A failed save must leave the previous ones in place.
    fn save(&self, allowed: &[AllowedAction]) -> std::io::Result<()>;
}
/// Grants in one JSON file, replaced by writing a sibling file and renaming it over.
pub struct FileGrants(PathBuf);
impl FileGrants {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }
}
impl GrantStore for FileGrants {
    fn load(&self) -> std::io::Result<Vec<AllowedAction>> {
        match std::fs::read(&self.0) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }
    fn save(&self, allowed: &[AllowedAction]) -> std::io::Result<()> {
        if let Some(parent) = self.0.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.0.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(allowed)?)?;
        std::fs::rename(tmp, &self.0)
    }
}
struct Pending {
    request: ApprovalRequest,
    deadline: Instant,
    shown: Option<Instant>,
    text: String,
}
#[derive(Default)]
struct State {
    custom_answers: std::collections::BTreeMap<String, (Answer, Instant)>,
    pending: VecDeque<Pending>,
    allowed: Vec<AllowedAction>,
    log: VecDeque<ActionLog>,
    serial: u64,
    load_failed: bool,
}
struct Inner {
    controls: OnceLock<Arc<crate::controls::DeviceControls>>,
    state: Mutex<State>,
    grant_writer: Mutex<()>,
    events: Events,
    clock: Clock,
    devices: DeviceManager,
    links: Links,
    grants: Arc<dyn GrantStore>,
    prompter: Arc<dyn ApprovalPrompter>,
    stopped: AtomicBool,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    starting: Mutex<()>,
    running: AtomicUsize,
    #[cfg(any(test, feature = "test-hooks"))]
    fake: Option<Arc<FakePrompter>>,
}
#[derive(Clone)]
pub struct Actions(Arc<Inner>);
impl Actions {
    pub(crate) fn new(
        events: Events,
        clock: Clock,
        devices: DeviceManager,
        links: Links,
        grants: Arc<dyn GrantStore>,
        mock: bool,
    ) -> Self {
        let mut state = State::default();
        match grants.load() {
            Ok(allowed) => state.allowed = allowed,
            Err(_) => state.load_failed = true,
        }
        let prompter: Arc<dyn ApprovalPrompter> = Arc::new(NativePrompter::default());
        #[cfg(any(test, feature = "test-hooks"))]
        let fake = (mock && std::env::var_os("TOUCHDECK_NATIVE_APPROVALS").is_none())
            .then(|| Arc::new(FakePrompter::default()));
        #[cfg(any(test, feature = "test-hooks"))]
        let prompter: Arc<dyn ApprovalPrompter> = fake.clone().map_or(prompter, |p| p);
        let _ = mock;
        let inner = Arc::new(Inner {
            controls: OnceLock::new(),
            state: Mutex::new(state),
            grant_writer: Mutex::new(()),
            events,
            clock,
            devices: devices.clone(),
            links,
            grants,
            prompter,
            stopped: AtomicBool::new(false),
            worker: Mutex::new(None),
            starting: Mutex::new(()),
            running: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-hooks"))]
            fake,
        });
        let weak = Arc::downgrade(&inner);
        devices.action_handler(Arc::new(move |device, event| {
            if let Some(inner) = weak.upgrade() {
                Actions(inner).press(device, event);
            }
        }));
        let weak = Arc::downgrade(&inner);
        *inner.worker.lock() = Some(std::thread::spawn(move || {
            loop {
                let Some(inner) = weak.upgrade() else { break };
                if inner.stopped.load(Ordering::SeqCst) {
                    break;
                }
                let actions = Actions(inner);
                actions.prompt_next();
                drop(actions);
                std::thread::sleep(Duration::from_millis(20));
            }
        }));
        Self(inner)
    }
    pub(crate) fn stop(&self) {
        self.0.stopped.store(true, Ordering::SeqCst);
        let pending = {
            let mut state = self.0.state.lock();
            state.custom_answers.clear();
            std::mem::take(&mut state.pending)
        };
        for pending in pending {
            self.0.events.publish(&ApprovalResolved {
                id: pending.request.id.clone(),
                answer: Answer::Expired,
            });
            if pending.request.spec.kind != "custom" {
                self.logged(
                    &pending.request,
                    "expired",
                    "Approval expired.".into(),
                    None,
                    None,
                );
            }
        }
    }
    /// Blocking: waits for the prompt to close and any process start already in progress.
    /// Commands that have already started keep running.
    pub(crate) fn wait_stopped(&self) {
        let worker = self.0.worker.lock().take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
        let _starting = self.0.starting.lock();
    }
    pub fn pending(&self) -> Vec<ApprovalRequest> {
        self.0
            .state
            .lock()
            .pending
            .iter()
            .map(|p| p.request.clone())
            .collect()
    }
    pub fn allowed(&self) -> Vec<AllowedAction> {
        self.0.state.lock().allowed.clone()
    }
    pub fn log(&self, device: Option<&DeviceId>) -> Vec<ActionLog> {
        self.0
            .state
            .lock()
            .log
            .iter()
            .filter(|l| device.is_none_or(|d| d == &l.device))
            .cloned()
            .collect()
    }
    fn change_grants(
        &self,
        change: impl FnOnce(&mut Vec<AllowedAction>) -> bool,
    ) -> std::io::Result<bool> {
        let _one_at_a_time = self.0.grant_writer.lock();
        let (mut next, load_failed) = {
            let state = self.0.state.lock();
            (state.allowed.clone(), state.load_failed)
        };
        if !change(&mut next) {
            return Ok(false);
        }
        if load_failed {
            return Err(std::io::Error::other(
                "The saved allowed actions could not be read.",
            ));
        }
        self.0.grants.save(&next)?;
        self.0.state.lock().allowed = next;
        Ok(true)
    }
    /// Blocking: call it off the async runtime.
    pub fn remove(&self, hash: &str) -> Result<bool, std::io::Error> {
        self.change_grants(|allowed| {
            let before = allowed.len();
            allowed.retain(|a| a.hash != hash);
            allowed.len() != before
        })
    }
    pub fn show(&self, id: &str) -> Result<(), crate::features::actions::ActionError> {
        use crate::{api::CommonError, features::actions::ActionError};
        let mut state = self.0.state.lock();
        let p = state
            .pending
            .iter_mut()
            .find(|p| p.request.id == id && self.0.clock.now() < p.deadline)
            .ok_or(ActionError::ApprovalMissing)?;
        let now = self.0.clock.now();
        if p.shown
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(5))
        {
            return Err(CommonError::Limits(
                "Show prompt again is limited to once every 5 seconds.",
            )
            .into());
        }
        p.shown = Some(now);
        self.0.prompter.show(id);
        Ok(())
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn script(&self, answers: Vec<prompt::ScriptAnswer>) -> bool {
        if let Some(fake) = &self.0.fake {
            fake.script(answers);
            true
        } else {
            false
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn prompted(&self) -> Vec<String> {
        self.0.fake.as_ref().map(|f| f.texts()).unwrap_or_default()
    }
    fn logged(
        &self,
        request: &ApprovalRequest,
        status: &str,
        detail: String,
        pid: Option<u32>,
        exit_code: Option<i32>,
    ) {
        let mut state = self.0.state.lock();
        state.serial += 1;
        let entry = ActionLog {
            id: format!("a{}", state.serial),
            device: request.device.clone(),
            at_ms: crate::now_ms(),
            page: request.page.clone(),
            tile: request.tile.clone(),
            kind: request.spec.kind.clone(),
            status: status.into(),
            detail,
            pid,
            exit_code,
        };
        self.0.events.publish(&entry);
        state.log.push_back(entry);
        while state.log.len() > 100 {
            state.log.pop_front();
        }
    }
    fn resolve(
        &self,
        device: &DeviceId,
        session: &td_link::Session,
        page: &str,
        tile: &str,
    ) -> Result<Option<(Tile, prompt::Context)>, String> {
        let shown = session
            .layout_state(Duration::from_secs(2))
            .map_err(|e| format!("The Device didn't report its Layout: {e}"))?;
        let bytes = match shown.source.as_str() {
            "builtin" => include_bytes!("../../../../../schema/examples/builtin.json").to_vec(),
            "storage" => {
                td_link::xfer::get_layout(session, shown.orientation)
                    .map_err(|e| format!("The Device's Layout couldn't be read: {e}"))?
                    .data
            }
            _ => return Ok(None),
        };
        let layout =
            Layout::parse(&bytes).map_err(|_| "The Device's Layout isn't valid.".to_owned())?;
        let Some(found) = td_host::actions::find_tile(&layout, page, tile) else {
            return Ok(None);
        };
        let title = layout
            .pages
            .iter()
            .find_map(|p| match p {
                Page::Grid { id, title, .. } if id == page => Some(title.as_str()),
                _ => None,
            })
            .unwrap_or(page);
        let name = self
            .0
            .devices
            .get(device)
            .and_then(|d| d.name)
            .unwrap_or_else(|| device.to_string());
        Ok(Some((
            found.clone(),
            prompt::Context {
                device: name,
                page: title.into(),
                tile: found.label.clone(),
            },
        )))
    }
    pub(crate) fn attach(
        &self,
        authority: crate::layouts::LayoutAuthority,
        controls: crate::features::integrations::Controls,
        settable: crate::controls::Settable,
        turned_on: crate::controls::TurnedOn,
    ) {
        let weak = Arc::downgrade(&self.0);
        let module = Arc::new(crate::controls::DeviceControls::new(
            authority,
            controls,
            settable,
            turned_on,
            self.0.events.clone(),
            Arc::new(move |request, status, detail| {
                if let Some(inner) = weak.upgrade() {
                    Actions(inner).logged(request, status, detail, None, None);
                }
            }),
        ));
        self.0.devices.controls(&module);
        let _ = self.0.controls.set(module);
    }
    fn press(&self, device: &DeviceId, event: &serde_json::Value) {
        if self.0.stopped.load(Ordering::SeqCst) {
            return;
        }
        let page = event["page"].as_str().unwrap_or_default();
        let tile = event["tile"].as_str().unwrap_or_default();
        let mut request = ApprovalRequest {
            id: String::new(),
            device: device.clone(),
            page: page.into(),
            tile: tile.into(),
            hash: String::new(),
            spec: ActionSpec {
                kind: event["kind"].as_str().unwrap_or_default().into(),
                program: String::new(),
                args: vec![],
                cwd: String::new(),
                shell: false,
            },
            created_ms: crate::now_ms(),
        };
        let resolved = match self.0.links.within(device, TAP_WAIT, |session| {
            self.resolve(device, session, page, tile)
        }) {
            Some(Ok(resolved)) => resolved,
            Some(Err(link)) => Err(link.to_string()),
            None => Err(
                "The Device stayed busy with a Layout or Asset transfer, so the tap was dropped."
                    .into(),
            ),
        };
        let (plan, context) = match resolved {
            Ok(Some((tile, context))) => (Plan::of(&tile.action), context),
            Ok(None) => {
                self.logged(
                    &request,
                    "unknown",
                    "No matching Tile in the Device's Layout.".into(),
                    None,
                    None,
                );
                return;
            }
            Err(reason) => {
                self.logged(&request, "failed", reason, None, None);
                return;
            }
        };
        let approval = match plan {
            Plan::Deck | Plan::Integration { .. } => return,
            Plan::Launch { path, args, cwd } => {
                request.spec = ActionSpec {
                    kind: "launch".into(),
                    program: path,
                    args,
                    cwd,
                    shell: false,
                };
                true
            }
            Plan::Command(spec) => {
                request.spec = ActionSpec {
                    kind: "command".into(),
                    program: spec.program,
                    args: spec.args,
                    cwd: spec.cwd,
                    shell: spec.shell,
                };
                true
            }
            Plan::Open { target } => {
                let safe = target.split_once(':').is_some_and(|(scheme, _)| {
                    ["http", "https", "mailto"]
                        .iter()
                        .any(|s| scheme.eq_ignore_ascii_case(s))
                }) && !target.chars().any(char::is_control);
                request.spec.kind = "open".into();
                request.spec.program = target;
                !safe
            }
        };
        request.hash = request.spec.hash();
        let text = prompt::dialog_text(&context, &request.spec);
        let mut state = self.0.state.lock();
        if self.0.stopped.load(Ordering::SeqCst) {
            return;
        }
        if state.pending.iter().any(|p| {
            p.request.device == *device && p.request.page == page && p.request.tile == tile
        }) {
            return;
        }
        if !approval || state.allowed.iter().any(|a| a.hash == request.hash) {
            drop(state);
            self.execute(request);
            return;
        }
        if state.pending.len() >= 16 {
            drop(state);
            self.logged(&request, "denied", "limits.exceeded".into(), None, None);
            return;
        }
        state.serial += 1;
        request.id = format!("p{}", state.serial);
        self.0.events.publish(&request);
        state.pending.push_back(Pending {
            request,
            deadline: self.0.clock.now() + Duration::from_secs(300),
            shown: None,
            text,
        });
    }
    fn prompt_next(&self) {
        let next = self
            .0
            .state
            .lock()
            .pending
            .front()
            .map(|p| (p.request.clone(), p.deadline, p.text.clone()));
        let Some((request, deadline, text)) = next else {
            return;
        };
        let expired = || {
            self.0.stopped.load(Ordering::SeqCst)
                || self.0.clock.now() >= deadline
                || Arc::strong_count(&self.0) == 1
        };
        let prompted = if expired() {
            Ok(Answer::Expired)
        } else {
            let _suppressed = self.0.devices.suppress_input();
            self.0.prompter.prompt(&request, &text, &expired)
        };
        let (answer, detail) = match prompted {
            _ if expired() => (Answer::Expired, "Approval expired.".to_owned()),
            Ok(Answer::Always) => {
                let grant = AllowedAction {
                    hash: request.hash.clone(),
                    spec: request.spec.clone(),
                    granted_ms: crate::now_ms(),
                };
                let saved = self.change_grants(|allowed| {
                    allowed.retain(|a| a.hash != grant.hash);
                    allowed.push(grant);
                    true
                });
                match saved {
                    Ok(_) => (Answer::Always, String::new()),
                    Err(e) => (
                        Answer::Deny,
                        format!("The allowed action couldn't be saved, so it was denied: {e}"),
                    ),
                }
            }
            Ok(Answer::Deny) => (Answer::Deny, "Denied on this PC.".to_owned()),
            Ok(answer) => (answer, String::new()),
            Err(e) => (
                Answer::Deny,
                format!("The approval prompt failed, so the action was denied: {e}"),
            ),
        };
        let mut state = self.0.state.lock();
        // `stop` may have already expired this request while its dialog was closing.
        if !state
            .pending
            .front()
            .is_some_and(|p| p.request.id == request.id)
        {
            return;
        }
        state.pending.pop_front();
        if request.spec.kind == "custom" {
            state
                .custom_answers
                .insert(request.hash.clone(), (answer, deadline));
        }
        self.0.events.publish(&ApprovalResolved {
            id: request.id.clone(),
            answer,
        });
        drop(state);
        if request.spec.kind == "custom" {
            return;
        }
        match answer {
            Answer::Once | Answer::Always => self.execute(request),
            Answer::Deny => self.logged(&request, "denied", detail, None, None),
            Answer::Expired => self.logged(&request, "expired", detail, None, None),
        }
    }
    fn execute(&self, request: ApprovalRequest) {
        if self.0.stopped.load(Ordering::SeqCst) {
            return;
        }
        if self.0.running.fetch_add(1, Ordering::SeqCst) >= 16 {
            self.0.running.fetch_sub(1, Ordering::SeqCst);
            self.logged(&request, "denied", "limits.exceeded".into(), None, None);
            return;
        }
        let actions = self.clone();
        std::thread::spawn(move || {
            let starting = actions.0.starting.lock();
            if actions.0.stopped.load(Ordering::SeqCst) {
                actions.0.running.fetch_sub(1, Ordering::SeqCst);
                return;
            }
            let spec = &request.spec;
            let result = match spec.kind.as_str() {
                "launch" => td_host::actions::launch(&spec.program, &spec.args, &spec.cwd)
                    .map(|pid| ("started", Some(pid), None)),
                "open" => td_host::actions::open(&spec.program).map(|()| ("opened", None, None)),
                _ => {
                    let child = td_host::actions::command(&spec.command())
                        .and_then(|mut command| command.spawn());
                    drop(starting);
                    child
                        .and_then(|mut child| child.wait())
                        .map(|exit| ("exited", None, exit.code()))
                }
            };
            match result {
                Ok((status, pid, exit)) => {
                    actions.logged(&request, status, spec.program.clone(), pid, exit)
                }
                Err(e) => actions.logged(&request, "failed", e.to_string(), None, None),
            }
            actions.0.running.fetch_sub(1, Ordering::SeqCst);
        });
    }
}
pub use prompt::ScriptAnswer;

impl td_host::integrations::CommandApproval for Actions {
    fn decision(&self, program: &str, args: &[String]) -> td_host::approval::CommandDecision {
        use td_host::approval::CommandDecision;
        let spec = ActionSpec {
            kind: "custom".into(),
            program: program.into(),
            args: args.to_vec(),
            cwd: String::new(),
            shell: false,
        };
        let hash = spec.hash();
        let mut state = self.0.state.lock();
        if self.0.stopped.load(Ordering::SeqCst) {
            return CommandDecision::Expired;
        }
        if state.allowed.iter().any(|a| a.hash == hash) {
            return CommandDecision::Approved;
        }
        match state.custom_answers.get(&hash).copied() {
            Some((answer @ (Answer::Once | Answer::Always), deadline)) => {
                state.custom_answers.remove(&hash);
                if answer == Answer::Once && self.0.clock.now() < deadline {
                    return CommandDecision::Approved;
                }
            }
            Some((Answer::Deny, _)) => return CommandDecision::Denied,
            Some((Answer::Expired, _)) => return CommandDecision::Expired,
            None => {}
        }
        if state.pending.iter().any(|p| p.request.hash == hash) || state.pending.len() >= 16 {
            return CommandDecision::Waiting;
        }
        while state.custom_answers.len() >= 256 {
            let oldest = state
                .custom_answers
                .iter()
                .min_by_key(|(_, (_, deadline))| *deadline)
                .map(|(hash, _)| hash.clone());
            if let Some(oldest) = oldest {
                state.custom_answers.remove(&oldest);
            }
        }
        state.serial += 1;
        let request = ApprovalRequest {
            id: format!("p{}", state.serial),
            device: DeviceId::parse("custom").expect("valid Integration context"),
            page: String::new(),
            tile: "custom".into(),
            hash,
            spec,
            created_ms: crate::now_ms(),
        };
        let text = prompt::dialog_text(
            &prompt::Context {
                device: "Custom Integration".into(),
                page: String::new(),
                tile: "Repeated command".into(),
            },
            &request.spec,
        );
        self.0.events.publish(&request);
        state.pending.push_back(Pending {
            request,
            deadline: self.0.clock.now() + Duration::from_secs(300),
            shown: None,
            text,
        });
        CommandDecision::Waiting
    }
}

#[cfg(test)]
mod custom_tests {
    use super::*;
    use td_host::integrations::CommandApproval;
    #[derive(Default)]
    struct Grants(Mutex<Vec<AllowedAction>>);
    impl GrantStore for Grants {
        fn load(&self) -> std::io::Result<Vec<AllowedAction>> {
            Ok(self.0.lock().clone())
        }
        fn save(&self, allowed: &[AllowedAction]) -> std::io::Result<()> {
            *self.0.lock() = allowed.to_vec();
            Ok(())
        }
    }
    fn service() -> Actions {
        let events = Events::default();
        let devices = DeviceManager::new(events.clone());
        Actions::new(
            events,
            Clock::default(),
            devices.clone(),
            Links::new(devices),
            Arc::new(Grants::default()),
            true,
        )
    }
    fn resolved(actions: &Actions) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !actions.pending().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(actions.pending().is_empty());
    }
    #[tokio::test]
    async fn shutdown_expires_open_approvals_and_refuses_execution() {
        use crate::companion::{Companion, Config};
        let dir = tempfile::tempdir().unwrap();
        let companion = Companion::start(Config::mock(dir.path().to_owned()))
            .await
            .unwrap();
        let handle = companion.handle();
        let actions = handle.actions();
        let request = ApprovalRequest {
            id: "p1".into(),
            device: DeviceId::parse("mock0001").unwrap(),
            page: "main".into(),
            tile: "command".into(),
            hash: String::new(),
            spec: ActionSpec {
                kind: "command".into(),
                program: if cfg!(windows) { "cmd" } else { "sh" }.into(),
                args: vec![
                    if cfg!(windows) { "/C" } else { "-c" }.into(),
                    "echo ran>runs".into(),
                ],
                cwd: dir.path().to_string_lossy().into_owned(),
                shell: false,
            },
            created_ms: crate::now_ms(),
        };
        actions.0.state.lock().pending.push_back(Pending {
            request: request.clone(),
            deadline: Instant::now() + Duration::from_secs(300),
            shown: None,
            text: "Approve command".into(),
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while actions.prompted().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut changes = handle.observe();
        companion.shutdown().await;
        assert!(
            actions.pending().is_empty(),
            "shutdown must expire an open prompt"
        );
        assert!(
            actions
                .log(None)
                .iter()
                .any(|entry| entry.status == "expired")
        );
        assert!(std::iter::from_fn(|| changes.try_recv().ok()).any(|change| matches!(
            change,
            crate::companion::Change::ApprovalResolved(resolved) if resolved.id == "p1" && resolved.answer == Answer::Expired
        )));
        actions.script(vec![ScriptAnswer::Always]);
        actions.execute(request);
        assert_eq!(actions.0.running.load(Ordering::SeqCst), 0);
        assert!(!dir.path().join("runs").exists());
        assert_eq!(
            actions.decision("example.exe", &[]),
            td_host::approval::CommandDecision::Expired
        );
        assert!(actions.pending().is_empty());
    }
    #[test]
    fn custom_approval_binds_arguments_denies_and_expires() {
        use td_host::approval::CommandDecision;
        let actions = service();
        actions.script(vec![ScriptAnswer::Deny]);
        assert_eq!(
            actions.decision("example.exe", &["original".into()]),
            CommandDecision::Waiting
        );
        resolved(&actions);
        assert_eq!(
            actions.decision("example.exe", &["original".into()]),
            CommandDecision::Denied
        );
        actions.script(vec![ScriptAnswer::Once]);
        assert_eq!(
            actions.decision("example.exe", &["changed".into()]),
            CommandDecision::Waiting
        );
        resolved(&actions);
        assert_eq!(
            actions.decision("example.exe", &["changed".into()]),
            CommandDecision::Approved
        );
        actions.script(vec![ScriptAnswer::Never]);
        assert_eq!(
            actions.decision("example.exe", &["changed".into()]),
            CommandDecision::Waiting
        );
        actions.0.clock.advance(Duration::from_secs(301));
        resolved(&actions);
        assert_eq!(
            actions.decision("example.exe", &["changed".into()]),
            CommandDecision::Expired
        );
        assert!(actions.log(None).is_empty());
    }
    #[test]
    fn custom_always_grant_is_exact_and_revocable() {
        use td_host::approval::CommandDecision;
        let actions = service();
        actions.script(vec![ScriptAnswer::Always]);
        assert_eq!(
            actions.decision("example.exe", &[]),
            CommandDecision::Waiting
        );
        resolved(&actions);
        assert_eq!(
            actions.decision("example.exe", &[]),
            CommandDecision::Approved
        );
        assert_eq!(
            actions.decision("example.exe", &[]),
            CommandDecision::Approved
        );
        let hash = actions.allowed()[0].hash.clone();
        actions.remove(&hash).unwrap();
        actions.script(vec![ScriptAnswer::Deny]);
        assert_eq!(
            actions.decision("example.exe", &[]),
            CommandDecision::Waiting
        );
        resolved(&actions);
        assert_eq!(
            actions.decision("example.exe", &[]),
            CommandDecision::Denied
        );
        assert!(actions.pending().is_empty());
        assert!(actions.log(None).is_empty());
    }
    #[test]
    fn custom_run_once_unused_past_its_deadline_asks_again() {
        use td_host::approval::CommandDecision;
        let actions = service();
        actions.script(vec![ScriptAnswer::Once]);
        assert_eq!(
            actions.decision("example.exe", &[]),
            CommandDecision::Waiting
        );
        resolved(&actions);
        actions.0.clock.advance(Duration::from_secs(301));
        actions.script(vec![ScriptAnswer::Never]);
        assert_eq!(
            actions.decision("example.exe", &[]),
            CommandDecision::Waiting
        );
        assert_eq!(actions.pending().len(), 1);
    }
    #[test]
    fn a_full_answer_list_drops_its_oldest_entry_so_new_commands_still_ask() {
        use td_host::approval::CommandDecision;
        let actions = service();
        actions.script(vec![ScriptAnswer::Deny]);
        assert_eq!(
            actions.decision("example.exe", &["0".into()]),
            CommandDecision::Waiting
        );
        resolved(&actions);
        for i in 1..256 {
            actions.0.clock.advance(Duration::from_secs(1));
            actions.0.state.lock().custom_answers.insert(
                format!("synthetic-{i}"),
                (
                    Answer::Deny,
                    actions.0.clock.now() + Duration::from_secs(300),
                ),
            );
        }
        actions.script(vec![ScriptAnswer::Never]);
        assert_eq!(
            actions.decision("example.exe", &["new".into()]),
            CommandDecision::Waiting
        );
        assert_eq!(actions.pending().len(), 1);
        assert_eq!(
            actions.decision("example.exe", &["0".into()]),
            CommandDecision::Waiting
        );
        assert_eq!(actions.pending().len(), 2);
        assert!(actions.0.state.lock().custom_answers.len() <= 256);
    }
}
