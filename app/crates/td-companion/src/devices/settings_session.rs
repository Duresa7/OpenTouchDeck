//! One Device's settings as a screen edits them: what it shows, what the Device last saved, which
//! fields wait for an answer, the orientation waiting for confirmation, and the restart that
//! applies it. Ported from the 1.x app's `lib/device/settings.ts`, so every screen (Settings and
//! setup) and every test crosses the same rules.
//!
//! A change shows at once. When the Device refuses it or doesn't answer, the field rolls back to
//! what the Device last saved, unless a newer change to the same field is still on its way. An
//! answer to one field never overwrites another field that is still pending.
use super::{
    DeviceError, DeviceId, DeviceSettings, DeviceSettingsPatch, LinkState, Operations,
    PhotoFramePatch, SettingsOutcome,
};
use crate::api::CommonError;
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use td_core::Orientation;
use tokio::sync::watch;

/// How long a restart may take before it counts as failed, if the Device went away.
pub const RESTART_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Load {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Restart {
    #[default]
    Idle,
    Restarting,
    Failed,
}

/// A field a screen changes on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Name,
    Orientation,
    PhotoFrame,
}
const FIELDS: [Field; 3] = [Field::Name, Field::Orientation, Field::PhotoFrame];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Ok,
    Error,
}

/// The one line under a Device's settings that says what just happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    /// The error behind it, for Details.
    pub detail: Option<String>,
    pub event: &'static str,
}

/// What the Device's Firmware can't change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    pub name: bool,
    pub photo_frame: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsView {
    pub load: Load,
    /// What the screen shows: the saved settings with the changes still on their way.
    pub shown: Option<DeviceSettings>,
    /// What the Device last said it keeps.
    pub saved: Option<DeviceSettings>,
    pub needs: Needs,
    pub name_problem: Option<String>,
    pub folder_problem: Option<String>,
    /// An orientation chosen and waiting for "Restart".
    pub confirm: Option<Orientation>,
    pub restart: Restart,
    pub notice: Option<Notice>,
}
impl SettingsView {
    /// The screen may change fields: loaded and not restarting.
    pub fn editable(&self) -> bool {
        self.load == Load::Ready && self.restart != Restart::Restarting
    }
}
pub fn orientation_of(name: &str) -> Option<Orientation> {
    match name {
        "landscape" => Some(Orientation::Landscape),
        "portrait" => Some(Orientation::Portrait),
        _ => None,
    }
}
pub fn orientation_name(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Landscape => "landscape",
        Orientation::Portrait => "portrait",
    }
}

const FOLDER_GONE: &str = "That folder isn't on this PC anymore. Choose another.";

#[derive(Default)]
struct Sequence {
    next: u64,
    latest: BTreeMap<Field, u64>,
    settled: BTreeMap<Field, u64>,
}
impl Sequence {
    fn pending(&self, field: Field) -> bool {
        self.latest.get(&field) != self.settled.get(&field)
    }
}

struct Inner {
    operations: Operations,
    id: DeviceId,
    view: watch::Sender<SettingsView>,
    sequence: Mutex<Sequence>,
    restart_timeout: Duration,
}

/// One Device's settings for the screens. Clone it freely: clones share their state.
#[derive(Clone)]
pub struct SettingsSession {
    inner: Arc<Inner>,
}

impl Field {
    /// The parts of a Device settings change this field asks for.
    fn settings_fields(self) -> &'static [super::SettingsField] {
        use super::SettingsField as S;
        match self {
            Field::Name => &[S::Name],
            Field::Orientation => &[S::Orientation],
            Field::PhotoFrame => &[S::PhotoFrame, S::PhotoFolder],
        }
    }
}

/// An answer's settings, keeping the photo frame already known when a failure came before the
/// answer read it: unread isn't "no photo frame".
fn known(before: Option<&DeviceSettings>, answer: &DeviceSettings) -> DeviceSettings {
    let mut settings = answer.clone();
    if settings.photo_frame.is_none()
        && let Some(frame) = before.and_then(|b| b.photo_frame.clone())
    {
        settings.photo_frame = Some(frame);
    }
    settings
}

fn take(settings: &mut DeviceSettings, from: &DeviceSettings, field: Field) {
    match field {
        Field::Name => settings.name.clone_from(&from.name),
        Field::Orientation => settings.orientation.clone_from(&from.orientation),
        Field::PhotoFrame => settings.photo_frame.clone_from(&from.photo_frame),
    }
}

impl SettingsSession {
    pub fn new(operations: Operations, id: DeviceId) -> Self {
        Self::with_restart_timeout(operations, id, RESTART_TIMEOUT)
    }
    pub fn with_restart_timeout(operations: Operations, id: DeviceId, timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                operations,
                id,
                view: watch::channel(SettingsView::default()).0,
                sequence: Mutex::default(),
                restart_timeout: timeout,
            }),
        }
    }
    pub fn id(&self) -> &DeviceId {
        &self.inner.id
    }
    pub fn view(&self) -> SettingsView {
        self.inner.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<SettingsView> {
        self.inner.view.subscribe()
    }
    fn set(&self, change: impl FnOnce(&mut SettingsView)) {
        self.inner.view.send_modify(change);
    }
    fn say(&self, notice: Option<Notice>) {
        self.set(|view| view.notice = notice);
    }
    fn called(&self) -> String {
        self.inner
            .operations
            .devices
            .get(&self.inner.id)
            .and_then(|device| device.name)
            .unwrap_or_else(|| "the Device".into())
    }
    fn connected(&self) -> Option<bool> {
        self.inner
            .operations
            .devices
            .get(&self.inner.id)
            .map(|device| matches!(device.link, LinkState::Connected { .. }))
    }

    /// Reads the settings from the Device, if it is connected.
    pub async fn load(&self) {
        if self.connected() != Some(true) {
            return;
        }
        self.set(|view| view.load = Load::Loading);
        match self.inner.operations.settings_get(&self.inner.id).await {
            Ok(settings) => {
                let protocol = self
                    .inner
                    .operations
                    .devices
                    .get(&self.inner.id)
                    .and_then(|device| device.firmware)
                    .map_or(1, |firmware| firmware.protocol);
                self.set(|view| {
                    view.load = Load::Ready;
                    view.needs = Needs {
                        name: protocol < 2,
                        photo_frame: settings.photo_frame.is_none(),
                    };
                    view.saved = Some(settings.clone());
                    view.shown = Some(settings);
                });
            }
            Err(error) => {
                let text = format!(
                    "Couldn't read {}'s settings. Unplug it and plug it back in.",
                    self.called()
                );
                self.set(|view| {
                    view.load = Load::Failed;
                    view.notice = Some(Notice {
                        kind: NoticeKind::Error,
                        text,
                        detail: Some(error.to_string()),
                        event: "read-failed",
                    });
                });
            }
        }
    }

    /// Shows `show` at once and sends `patch`. On a refusal or no answer the field returns to what
    /// the Device saved, unless a newer change of it is on its way.
    async fn patch(
        &self,
        field: Field,
        show: impl FnOnce(&mut DeviceSettings),
        patch: DeviceSettingsPatch,
        quiet: bool,
    ) -> Option<SettingsOutcome> {
        let mine = {
            let mut sequence = self.inner.sequence.lock();
            sequence.next += 1;
            let mine = sequence.next;
            sequence.latest.insert(field, mine);
            mine
        };
        self.set(|view| {
            if let Some(shown) = &mut view.shown {
                show(shown);
            }
            if !quiet {
                view.notice = None;
            }
        });
        let result = self
            .inner
            .operations
            .settings_patch(&self.inner.id, patch)
            .await;
        let (outcome, error) = match result {
            // A failure after this field was acknowledged doesn't undo it: the Device keeps the
            // change, so it counts as saved (`SettingsOutcome` lists what is still pending).
            Ok(outcome)
                if !field
                    .settings_fields()
                    .iter()
                    .any(|f| outcome.pending.contains(f)) =>
            {
                (Some(outcome), None)
            }
            Ok(outcome) => {
                let error = outcome.error.as_ref().map(|error| match error {
                    DeviceError::NeedsFirmware { field } => DeviceError::NeedsFirmware { field },
                    other => DeviceError::Link {
                        reason: other.to_string(),
                    },
                });
                (Some(outcome), error)
            }
            Err(error) => (None, Some(error)),
        };
        if let Some(error) = error {
            let latest = {
                let mut sequence = self.inner.sequence.lock();
                let latest = sequence.latest.get(&field) == Some(&mine);
                if latest {
                    sequence.settled.insert(field, mine);
                }
                latest
            };
            self.set(|view| {
                // What the Device acknowledged before the failure is what it keeps now.
                if let Some(outcome) = &outcome {
                    view.saved = Some(known(view.saved.as_ref(), &outcome.settings));
                }
                if latest && let (Some(shown), Some(saved)) = (&mut view.shown, &view.saved) {
                    take(shown, saved, field);
                }
            });
            match (&error, field) {
                (DeviceError::NeedsFirmware { .. }, Field::Name) => {
                    self.set(|view| view.needs.name = true)
                }
                (DeviceError::NeedsFirmware { .. }, Field::PhotoFrame) => {
                    self.set(|view| view.needs.photo_frame = true)
                }
                (DeviceError::Common(CommonError::Invalid { reason, .. }), Field::PhotoFrame)
                    if reason.contains("folder") =>
                {
                    self.set(|view| view.folder_problem = Some(FOLDER_GONE.into()))
                }
                _ => {
                    let text = format!(
                        "Couldn't save to {}. Check that it's plugged in, then retry.",
                        self.called()
                    );
                    self.say(Some(Notice {
                        kind: NoticeKind::Error,
                        text,
                        detail: Some(error.to_string()),
                        event: "save-failed",
                    }));
                }
            }
            return None;
        }
        let outcome = outcome.expect("an answer without an error");
        let pending: Vec<Field> = {
            let mut sequence = self.inner.sequence.lock();
            let settled = sequence.settled.entry(field).or_default();
            *settled = (*settled).max(mine);
            FIELDS
                .into_iter()
                .filter(|field| sequence.pending(*field))
                .collect()
        };
        let answered = known(self.view().saved.as_ref(), &outcome.settings);
        self.set(|view| {
            let mut shown = answered.clone();
            if let Some(before) = &view.shown {
                for field in &pending {
                    take(&mut shown, before, *field);
                }
            }
            view.saved = Some(answered);
            view.shown = Some(shown);
        });
        if outcome.restart_required && field != Field::Orientation {
            let text = format!("Saved. Restart {} to apply it.", self.called());
            self.say(Some(Notice {
                kind: NoticeKind::Info,
                text,
                detail: None,
                event: "restart-required",
            }));
        } else if !quiet {
            self.say(Some(saved()));
        }
        Some(outcome)
    }

    /// Checks a new name and sends it. `false` when it was refused, here or by the Device.
    pub async fn rename(&self, next: &str) -> bool {
        let view = self.view();
        let Some(shown) = view.shown else {
            return false;
        };
        if view.needs.name {
            return false;
        }
        let problem = super::name::name_problem(next);
        self.set(|view| view.name_problem = problem.map(|p| p.settings_text().into()));
        if problem.is_some() {
            return false;
        }
        if shown.name.as_deref() == Some(next) {
            return true;
        }
        let name = next.to_owned();
        self.patch(
            Field::Name,
            |shown| shown.name = Some(name.clone()),
            DeviceSettingsPatch {
                name: Some(name.clone()),
                ..Default::default()
            },
            false,
        )
        .await
        .is_some()
    }

    /// Chooses an orientation. A different one waits for `confirm_orientation`.
    pub fn choose_orientation(&self, orientation: Orientation) {
        self.set(|view| {
            let current = view
                .shown
                .as_ref()
                .and_then(|shown| orientation_of(&shown.orientation));
            view.confirm = (current != Some(orientation)).then_some(orientation);
        });
    }

    pub fn cancel_orientation(&self) {
        self.set(|view| view.confirm = None);
    }

    /// Saves the chosen orientation and, when the Device needs it, restarts it and waits.
    ///
    /// Returns whether the Device now keeps the orientation: saved, and restarted when it needed
    /// a restart. With nothing to confirm (it already shows it) that is `true`. Callers branch
    /// on this, not on the notice, which reads the same for the same failure twice.
    pub async fn confirm_orientation(&self) -> bool {
        let Some(orientation) = self.view().confirm else {
            return true;
        };
        self.set(|view| view.confirm = None);
        let name = orientation_name(orientation);
        let reply = self
            .patch(
                Field::Orientation,
                |shown| shown.orientation = name.into(),
                DeviceSettingsPatch {
                    orientation: Some(name.into()),
                    ..Default::default()
                },
                true,
            )
            .await;
        match reply {
            Some(reply) if reply.restart_required => self.restart().await,
            Some(_) => {
                self.say(Some(saved()));
                true
            }
            None => false,
        }
    }

    /// Changes the photo frame. Unset parts of `change` stay as they are.
    pub async fn set_photo_frame(&self, change: PhotoFramePatch) -> bool {
        let view = self.view();
        let Some(current) = view.shown.and_then(|shown| shown.photo_frame) else {
            return false;
        };
        if view.needs.photo_frame {
            return false;
        }
        self.set(|view| view.folder_problem = None);
        let mut next = current;
        if let Some(enabled) = change.enabled {
            next.enabled = enabled;
        }
        if let Some(timeout) = change.timeout_s {
            next.timeout_s = timeout;
        }
        if let Some(interval) = change.interval_s {
            next.interval_s = interval;
        }
        if let Some(folder) = &change.folder {
            next.folder = (!folder.is_empty()).then(|| folder.clone());
        }
        self.patch(
            Field::PhotoFrame,
            |shown| shown.photo_frame = Some(next),
            DeviceSettingsPatch {
                photo_frame: Some(change),
                ..Default::default()
            },
            false,
        )
        .await
        .is_some()
    }

    /// Restarts the Device and waits until it is back, then reads its settings again. It fails
    /// when the Device went away and didn't come back in time.
    ///
    /// Returns whether the Device came back. `false` when a restart was already under way.
    pub async fn restart(&self) -> bool {
        if self.view().restart == Restart::Restarting {
            return false;
        }
        self.set(|view| {
            view.restart = Restart::Restarting;
            view.notice = Some(Notice {
                kind: NoticeKind::Info,
                text: "Restarting…".into(),
                detail: None,
                event: "restarting",
            });
        });
        let failed = |detail: Option<String>| Notice {
            kind: NoticeKind::Error,
            text: format!(
                "Couldn't restart {}. Unplug it and plug it back in.",
                self.called()
            ),
            detail,
            event: "restart-failed",
        };
        let sent = match self.inner.operations.devices.restart(&self.inner.id).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(DeviceError::from(error).to_string()),
            Err(reach) => Err(DeviceError::from(reach).to_string()),
        };
        if let Err(detail) = sent {
            let notice = failed(Some(detail));
            self.set(|view| {
                view.restart = Restart::Failed;
                view.notice = Some(notice);
            });
            return false;
        }
        let deadline = tokio::time::Instant::now() + self.inner.restart_timeout;
        let mut dropped = false;
        let back = loop {
            match self.connected() {
                Some(false) | None => dropped = true,
                Some(true) if dropped => break true,
                Some(true) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                // A Device that never went away restarted too fast to notice.
                break !dropped;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        if back {
            self.set(|view| {
                view.restart = Restart::Idle;
                view.notice = None;
            });
            self.load().await;
        } else {
            let notice = failed(None);
            self.set(|view| {
                view.restart = Restart::Failed;
                view.notice = Some(notice);
            });
        }
        back
    }
}

fn saved() -> Notice {
    Notice {
        kind: NoticeKind::Ok,
        text: "Saved".into(),
        detail: None,
        event: "saved",
    }
}

/// The "Start after" choices for the photo frame, with a time set some other way in order.
pub const START_AFTER_S: [u32; 8] = [30, 60, 120, 300, 600, 900, 1800, 3600];

pub fn start_after_choices(current: u32) -> Vec<u32> {
    let mut choices = START_AFTER_S.to_vec();
    if !choices.contains(&current) {
        choices.push(current);
        choices.sort_unstable();
    }
    choices
}

/// A time as the photo frame shows it: "30 s", "5 min", "1 hr".
pub fn duration_label(seconds: u32) -> String {
    if seconds >= 3600 && seconds.is_multiple_of(3600) {
        format!("{} hr", seconds / 3600)
    } else if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}
