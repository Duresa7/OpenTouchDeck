//! First-run setup: which Device to set up, the steps it takes, the setups the person skipped,
//! and which New boards to show. Ported from the 1.x app's `lib/device/onboarding.ts`,
//! `setup-trigger.ts`, `state/setup.ts` and `screens/new-boards.ts`.
//!
//! A New board becomes a Device once Firmware is installed; the Companion then marks it as
//! needing setup until it is named. Setup opens by itself for the first such Device that is
//! connected and not skipped. Closing setup before the end skips it, so it doesn't open again
//! by itself; a new install on the same board clears the skip.
use crate::devices::{
    DeviceError, DeviceId, DeviceSettingsPatch, DeviceSummary, LinkState, NewBoard, NoticeKind,
    Operations, Restart, SettingsSession, SetupState, name_problem,
};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use td_core::Orientation;
use tokio::sync::watch;

/// One step of setup. "Pick a starting Layout" and the radio build's Wi-Fi step come with the
/// native Layout editor (M3); the radio build stays off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Firmware,
    Name,
    Orientation,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub steps: Vec<Step>,
    pub at: Step,
}

fn steps_of(firmware: bool, firmware_first: bool) -> Vec<Step> {
    let setup = [Step::Name, Step::Orientation];
    let mut steps = match (firmware, firmware_first) {
        (false, _) => setup.to_vec(),
        (true, true) => [&[Step::Firmware][..], &setup].concat(),
        (true, false) => [&setup[..], &[Step::Firmware]].concat(),
    };
    steps.push(Step::Done);
    steps
}

impl Flow {
    /// Firmware first when the Device's Firmware is too old to keep a name (protocol 1).
    pub fn start(firmware: bool, firmware_first: bool) -> Self {
        let steps = steps_of(firmware, firmware_first);
        Self {
            at: steps[0],
            steps,
        }
    }
    fn index(&self) -> usize {
        self.steps.iter().position(|s| *s == self.at).unwrap_or(0)
    }
    pub fn next(&mut self) {
        self.at = self.steps[(self.index() + 1).min(self.steps.len() - 1)];
    }
    pub fn back(&mut self) {
        self.at = self.steps[self.index().saturating_sub(1)];
    }
    /// Newer Firmware became known after setup opened: add its step.
    pub fn offer(&mut self, firmware_first: bool) {
        if self.steps.contains(&Step::Firmware) {
            return;
        }
        let first = firmware_first && self.index() == 0;
        self.steps = steps_of(true, first);
        if first {
            self.at = Step::Firmware;
        }
    }
    /// "Step 2 of 4".
    pub fn count(&self) -> (usize, usize) {
        (self.index() + 1, self.steps.len())
    }
    pub fn first(&self) -> bool {
        self.index() == 0
    }
}

/// The Device setup opens for by itself: the first one needing setup, connected and not skipped.
pub fn candidate<'a>(
    devices: &'a [DeviceSummary],
    skipped: &BTreeSet<String>,
) -> Option<&'a DeviceSummary> {
    devices.iter().find(|d| {
        d.setup == SetupState::NeedsSetup
            && matches!(d.link, LinkState::Connected { .. })
            && !skipped.contains(d.id.as_str())
    })
}

/// The New board cards: those shown and those ignored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardCards {
    pub shown: Vec<NewBoard>,
    pub ignored: Vec<NewBoard>,
}

/// A board whose install started stays shown until it becomes a Device, even while it is gone
/// from the port list as it restarts. An ignored board waits in its own list.
pub fn board_cards(
    boards: &[NewBoard],
    devices: &[DeviceSummary],
    ignored: &BTreeSet<DeviceId>,
    started: &BTreeMap<DeviceId, NewBoard>,
) -> BoardCards {
    let known: BTreeSet<&DeviceId> = devices.iter().map(|d| &d.id).collect();
    let listed: BTreeSet<&DeviceId> = boards.iter().map(|b| &b.id).collect();
    let mut shown: Vec<NewBoard> = boards
        .iter()
        .filter(|b| !ignored.contains(&b.id) || started.contains_key(&b.id))
        .cloned()
        .collect();
    shown.extend(
        started
            .values()
            .filter(|b| !known.contains(&b.id) && !listed.contains(&b.id))
            .cloned(),
    );
    shown.retain(|b| !known.contains(&b.id));
    BoardCards {
        shown,
        ignored: boards
            .iter()
            .filter(|b| {
                ignored.contains(&b.id) && !started.contains_key(&b.id) && !known.contains(&b.id)
            })
            .cloned()
            .collect(),
    }
}

/// The setups the person closed before the end, kept on this PC.
#[derive(Clone)]
pub struct Skips {
    path: Option<PathBuf>,
    skipped: Arc<Mutex<BTreeSet<String>>>,
}
impl Skips {
    /// Keeps the list in `setup-skipped.json` in this folder. A missing or damaged file is an
    /// empty list: at worst setup opens once more.
    pub fn load(data: &Path) -> Self {
        let path = data.join("setup-skipped.json");
        let skipped = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).ok())
            .map(|ids| {
                ids.into_iter()
                    .filter_map(|id| id.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            path: Some(path),
            skipped: Arc::new(Mutex::new(skipped)),
        }
    }
    /// Kept in memory only.
    pub fn memory() -> Self {
        Self {
            path: None,
            skipped: Default::default(),
        }
    }
    pub fn skipped(&self) -> BTreeSet<String> {
        self.skipped.lock().clone()
    }
    fn save(&self, skipped: &BTreeSet<String>) {
        let Some(path) = &self.path else { return };
        let bytes = serde_json::to_vec(skipped).expect("a list of strings");
        if let Err(error) = td_core::app_dirs::write_atomic(path, &bytes) {
            eprintln!("Companion: couldn't save {}: {error}", path.display());
        }
    }
    /// Setup closed. A Device still needing setup is skipped, so setup doesn't open again by itself.
    pub fn closed(&self, device: Option<&DeviceSummary>) {
        let Some(device) = device.filter(|d| d.setup == SetupState::NeedsSetup) else {
            return;
        };
        let mut skipped = self.skipped.lock();
        if skipped.insert(device.id.as_str().to_owned()) {
            self.save(&skipped);
        }
    }
    /// Firmware was installed on this board again, so its setup may open by itself again.
    pub fn unskip(&self, id: &DeviceId) {
        let mut skipped = self.skipped.lock();
        if skipped.remove(id.as_str()) {
            self.save(&skipped);
        }
    }
}

/// What setup is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Saving,
    Restarting,
}
impl Busy {
    pub fn label(self) -> &'static str {
        match self {
            Busy::Saving => "Saving…",
            Busy::Restarting => "Restarting…",
        }
    }
}

/// A failure with its Details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub text: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupView {
    pub flow: Flow,
    pub name: String,
    pub name_error: Option<String>,
    /// The Firmware can't keep a name, so the name step only moves on.
    pub needs_firmware: bool,
    pub orientation: Orientation,
    pub busy: Option<Busy>,
    pub error: Option<Problem>,
}

const PLUG_IN: &str = "Check that it's plugged in, then retry.";

fn orientation_of(device: &DeviceSummary) -> Orientation {
    match device.orientation {
        Some(Orientation::Portrait) => Orientation::Portrait,
        _ => Orientation::Landscape,
    }
}
fn v1(device: &DeviceSummary) -> bool {
    device.firmware.as_ref().map_or(1, |f| f.protocol) < 2
}

struct Inner {
    operations: Operations,
    settings: SettingsSession,
    id: DeviceId,
    view: watch::Sender<SetupView>,
    device: Mutex<DeviceSummary>,
}

/// One Device's setup. Clones share their state.
#[derive(Clone)]
pub struct SetupSession {
    inner: Arc<Inner>,
}

impl SetupSession {
    /// `firmware_offered`: this Companion carries newer Firmware for the Device.
    pub fn new(operations: Operations, device: DeviceSummary, firmware_offered: bool) -> Self {
        let settings = operations.settings_session(&device.id);
        let view = SetupView {
            flow: Flow::start(firmware_offered, v1(&device)),
            name: device.name.clone().unwrap_or_default(),
            name_error: None,
            needs_firmware: false,
            orientation: orientation_of(&device),
            busy: None,
            error: None,
        };
        Self {
            inner: Arc::new(Inner {
                operations,
                settings,
                id: device.id.clone(),
                view: watch::channel(view).0,
                device: Mutex::new(device),
            }),
        }
    }
    pub fn id(&self) -> &DeviceId {
        &self.inner.id
    }
    pub fn view(&self) -> SetupView {
        self.inner.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<SetupView> {
        self.inner.view.subscribe()
    }
    fn set(&self, change: impl FnOnce(&mut SetupView)) {
        self.inner.view.send_modify(change);
    }
    fn called(&self) -> String {
        self.inner
            .device
            .lock()
            .name
            .clone()
            .unwrap_or_else(|| "the Device".into())
    }
    fn step(&self, forward: bool) {
        self.set(|view| {
            if forward {
                view.flow.next();
            } else {
                view.flow.back();
            }
            view.error = None;
        });
    }

    pub fn set_name(&self, name: &str) {
        self.set(|view| {
            view.name = name.into();
            view.name_error = None;
            view.error = None;
        });
    }

    /// Checks the name and stores it on the Device, then moves on.
    pub async fn submit_name(&self) {
        let view = self.view();
        if view.busy.is_some() {
            return;
        }
        if view.needs_firmware {
            return self.step(true);
        }
        let name = view.name.trim().to_owned();
        if let Some(problem) = name_problem(&name) {
            return self.set(|view| view.name_error = Some(problem.setup_text()));
        }
        if self.inner.device.lock().name.as_deref() == Some(name.as_str()) {
            return self.step(true);
        }
        self.set(|view| {
            view.busy = Some(Busy::Saving);
            view.error = None;
        });
        let result = self
            .inner
            .operations
            .settings_patch(
                &self.inner.id,
                DeviceSettingsPatch {
                    name: Some(name.clone()),
                    ..Default::default()
                },
            )
            .await
            .and_then(|outcome| outcome.error.map_or(Ok(()), Err));
        match result {
            Ok(()) => {
                self.set(|view| view.name = name);
                self.step(true);
            }
            Err(DeviceError::NeedsFirmware { .. }) => self.set(|view| view.needs_firmware = true),
            Err(error) => {
                let text = format!("Couldn't save to {}. {PLUG_IN}", self.called());
                self.set(|view| {
                    view.error = Some(Problem {
                        text,
                        detail: error.to_string(),
                    })
                });
            }
        }
        self.set(|view| view.busy = None);
    }

    pub fn choose_orientation(&self, orientation: Orientation) {
        self.set(|view| {
            if view.busy.is_none() {
                view.orientation = orientation;
                view.error = None;
            }
        });
    }

    /// Keeps the orientation, or saves a new one and restarts the Device, then moves on.
    pub async fn submit_orientation(&self) {
        let view = self.view();
        if view.busy.is_some() {
            return;
        }
        let orientation = view.orientation;
        if orientation == orientation_of(&self.inner.device.lock()) {
            return self.step(true);
        }
        self.set(|view| {
            view.busy = Some(Busy::Saving);
            view.error = None;
        });
        let settings = &self.inner.settings;
        let before = settings.view().notice;
        settings.choose_orientation(orientation);
        let mut restarts = settings.subscribe();
        let me = self.clone();
        let watching = tokio::spawn(async move {
            while restarts.changed().await.is_ok() {
                let restarting = restarts.borrow().restart == Restart::Restarting;
                if restarting {
                    me.set(|view| view.busy = Some(Busy::Restarting));
                }
            }
        });
        settings.confirm_orientation().await;
        watching.abort();
        self.set(|view| view.busy = None);
        let notice = settings.view().notice;
        if notice != before
            && let Some(notice) = notice.filter(|n| n.kind == NoticeKind::Error)
        {
            return self.set(|view| {
                view.error = Some(Problem {
                    text: notice.text,
                    detail: notice.detail.unwrap_or_default(),
                })
            });
        }
        self.step(true);
    }

    /// Newer Firmware became known: add its step.
    pub fn offer_firmware(&self) {
        let first = v1(&self.inner.device.lock());
        self.set(|view| view.flow.offer(first));
    }

    pub fn next(&self) {
        if self.view().busy.is_none() {
            self.step(true);
        }
    }

    pub fn back(&self) {
        if self.view().busy.is_none() {
            self.step(false);
        }
    }

    /// Follows the Device: once its Firmware keeps names, the name step works again.
    pub fn observe(&self, device: &DeviceSummary) {
        *self.inner.device.lock() = device.clone();
        if !v1(device) && self.view().needs_firmware {
            self.set(|view| view.needs_firmware = false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{BoardProbe, ControlLayout, ControlLayoutUnavailable, KeyMirrorState};

    fn device(id: &str, setup: SetupState, connected: bool) -> DeviceSummary {
        DeviceSummary {
            id: DeviceId::parse(id).unwrap(),
            rev: 1,
            name: None,
            setup,
            link: if connected {
                LinkState::Connected {
                    port: "p".into(),
                    since_ms: 0,
                    protocol: 2,
                }
            } else {
                LinkState::Offline
            },
            carrier: None,
            firmware: None,
            diagnostics: None,
            capabilities: None,
            orientation: None,
            layout: None,
            control_layout: ControlLayout::Unavailable {
                reason: ControlLayoutUnavailable::Offline,
            },
            key_mirror: KeyMirrorState {
                enabled: false,
                reason: None,
            },
            stable_id: true,
        }
    }
    fn board(id: &str) -> NewBoard {
        NewBoard {
            id: DeviceId::parse(id).unwrap(),
            port: format!("mock:{id}"),
            probe: BoardProbe::Silent,
            seen_ms: 0,
        }
    }
    fn ids(boards: &[NewBoard]) -> Vec<&str> {
        boards.iter().map(|b| b.id.as_str()).collect()
    }

    #[test]
    fn steps_put_firmware_first_only_for_firmware_that_cant_keep_a_name() {
        use Step::*;
        assert_eq!(Flow::start(false, true).steps, [Name, Orientation, Done]);
        assert_eq!(
            Flow::start(true, true).steps,
            [Firmware, Name, Orientation, Done]
        );
        assert_eq!(
            Flow::start(true, false).steps,
            [Name, Orientation, Firmware, Done]
        );
    }

    #[test]
    fn next_and_back_stop_at_the_ends() {
        let mut flow = Flow::start(false, false);
        flow.back();
        assert_eq!(flow.at, Step::Name);
        assert_eq!(flow.count(), (1, 3));
        for _ in 0..5 {
            flow.next();
        }
        assert_eq!(flow.at, Step::Done);
        assert_eq!(flow.count(), (3, 3));
    }

    #[test]
    fn firmware_offered_later_joins_first_only_on_the_first_step() {
        let mut flow = Flow::start(false, false);
        flow.offer(true);
        assert_eq!(flow.at, Step::Firmware);
        assert_eq!(flow.steps[0], Step::Firmware);

        let mut flow = Flow::start(false, false);
        flow.next();
        flow.offer(true);
        assert_eq!(flow.at, Step::Orientation);
        assert_eq!(flow.steps[2], Step::Firmware);

        let mut offered = Flow::start(true, false);
        let before = offered.clone();
        offered.offer(true);
        assert_eq!(offered, before, "offered once");
    }

    #[test]
    fn setup_opens_for_a_connected_device_needing_it_unless_skipped() {
        let devices = [
            device("a1", SetupState::Ready, true),
            device("b2", SetupState::NeedsSetup, false),
            device("c3", SetupState::NeedsSetup, true),
            device("d4", SetupState::NeedsSetup, true),
        ];
        let mut skipped = BTreeSet::new();
        assert_eq!(candidate(&devices, &skipped).unwrap().id.as_str(), "c3");
        skipped.insert("c3".into());
        assert_eq!(candidate(&devices, &skipped).unwrap().id.as_str(), "d4");
        skipped.insert("d4".into());
        assert!(candidate(&devices, &skipped).is_none());
    }

    #[test]
    fn closing_setup_skips_it_and_a_new_install_clears_the_skip() {
        let dir = tempfile::tempdir().unwrap();
        let skips = Skips::load(dir.path());
        let needs = device("c3", SetupState::NeedsSetup, true);
        skips.closed(Some(&device("a1", SetupState::Ready, true)));
        skips.closed(None);
        assert!(skips.skipped().is_empty(), "nothing to skip");
        skips.closed(Some(&needs));
        assert_eq!(Skips::load(dir.path()).skipped(), ["c3".to_owned()].into());
        skips.unskip(&needs.id);
        assert!(Skips::load(dir.path()).skipped().is_empty());

        std::fs::write(dir.path().join("setup-skipped.json"), b"not json").unwrap();
        assert!(Skips::load(dir.path()).skipped().is_empty());
        std::fs::write(dir.path().join("setup-skipped.json"), br#"["x1", 3]"#).unwrap();
        assert_eq!(Skips::load(dir.path()).skipped(), ["x1".to_owned()].into());
    }

    #[test]
    fn board_cards_keep_an_install_until_it_is_a_device() {
        let a = board("a1");
        let b = board("b2");
        let mut ignored = BTreeSet::new();
        let mut started = BTreeMap::new();
        let cards = board_cards(&[a.clone(), b.clone()], &[], &ignored, &started);
        assert_eq!(ids(&cards.shown), ["a1", "b2"]);

        ignored.insert(b.id.clone());
        let cards = board_cards(&[a.clone(), b.clone()], &[], &ignored, &started);
        assert_eq!(ids(&cards.shown), ["a1"]);
        assert_eq!(ids(&cards.ignored), ["b2"]);

        // An install restarts the board: it leaves the port list but its card stays.
        started.insert(a.id.clone(), a.clone());
        let cards = board_cards(std::slice::from_ref(&b), &[], &ignored, &started);
        assert_eq!(ids(&cards.shown), ["a1"]);

        // Now it is a Device.
        let cards = board_cards(
            std::slice::from_ref(&b),
            &[device("a1", SetupState::NeedsSetup, true)],
            &ignored,
            &started,
        );
        assert!(cards.shown.is_empty());
        assert_eq!(ids(&cards.ignored), ["b2"]);
    }
}
