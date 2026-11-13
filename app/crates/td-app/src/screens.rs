//! Setup and Settings, fed from the Companion. Every rule they follow lives in td-companion
//! (`setup`, the Device settings and Pairings sessions, Firmware tasks) or td-core (the update
//! setting); this module only turns those views into the window's properties and sends the
//! window's callbacks back as typed operations.
//!
//! The Companion's work runs on Tokio. Each session publishes its view on a `watch` channel; a
//! small task per channel wakes the UI thread, which then rebuilds the window's properties from
//! the current views in [`Screens::refresh`]. The headless tests call `refresh` themselves.
use crate::{
    AppWindow, BoardCard, DeviceEntry, DevicePage, Page, PairedApp, PcSettings, SetupLater, Wizard,
    home, prefs::Appearance, window,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use td_companion::{
    companion::{Change, Handle, RecvError},
    devices::{
        DeviceId, DeviceSummary, Forgetting, LinkState, NewBoard, NoticeKind, PairingsSession,
        PhotoFramePatch, Restart, SettingsSession, SetupState, duration_label, offers_pairings,
        orientation_name, orientation_of, over_usb, start_after_choices,
    },
    features::firmware::{
        BoardStep, FirmwareBuild, FirmwareTask, JobView, Target, board_step, offered,
    },
    setup::{SetupSession, Step, board_cards, candidate},
};
use td_core::{Orientation, app_dirs::Dirs, settings::AppSettings};

/// Asks the person for a folder. Blocking; it runs off the UI thread.
pub(crate) type FolderPicker = Arc<dyn Fn() -> Result<Option<String>, String> + Send + Sync>;

thread_local! {
    static SCREENS: RefCell<Weak<Screens>> = const { RefCell::new(Weak::new()) };
}
static WAKING: AtomicBool = AtomicBool::new(false);

/// Asks the UI thread to refresh once, however many changes arrive before it does.
fn wake() {
    if WAKING.swap(true, Ordering::AcqRel) {
        return;
    }
    let posted = slint::invoke_from_event_loop(|| {
        WAKING.store(false, Ordering::Release);
        if let Some(screens) = SCREENS.with(|s| s.borrow().upgrade()) {
            screens.refresh();
        }
    });
    if posted.is_err() {
        WAKING.store(false, Ordering::Release);
    }
}

/// "Desk" → "Desk"; "the Device" → "The Device".
fn opening(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn called(device: &DeviceSummary) -> String {
    device.name.clone().unwrap_or_else(|| "the Device".into())
}

fn shown_name(device: &DeviceSummary) -> String {
    home::row(device).name.into()
}

fn connected(device: &DeviceSummary) -> bool {
    matches!(device.link, LinkState::Connected { .. })
}

fn needs_firmware_text(name: &str) -> String {
    format!("{name}'s Firmware can't change this yet.")
}

/// Whether Start at sign-in can be read and what it is.
#[derive(Clone, Debug, PartialEq)]
enum StartAtSignIn {
    Reading,
    Ready { on: bool, problem: Option<String> },
    Unreadable,
}

#[derive(Clone, Debug, PartialEq)]
enum UpdateChecks {
    Ready { on: bool, problem: Option<String> },
    Unreadable(String),
}

const START_READ_PROBLEM: &str =
    "Couldn't read Start at sign-in. Restart Open TouchDeck, then retry.";

/// Mirrors a list into a model in place, so rows that didn't change keep their elements, and
/// with them keyboard focus.
fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    for (index, row) in rows.iter().enumerate() {
        if index < model.row_count() {
            if model.row_data(index).as_ref() != Some(row) {
                model.set_row_data(index, row.clone());
            }
        } else {
            model.push(row.clone());
        }
    }
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
}

#[derive(Default)]
struct Models {
    boards: Rc<VecModel<BoardCard>>,
    ignored: Rc<VecModel<SharedString>>,
    later: Rc<VecModel<SetupLater>>,
    devices: Rc<VecModel<DeviceEntry>>,
    paired: Rc<VecModel<PairedApp>>,
    after: Rc<VecModel<SharedString>>,
}

pub(crate) struct Screens {
    ui: slint::Weak<AppWindow>,
    handle: Handle,
    runtime: tokio::runtime::Handle,
    window: Rc<window::State>,
    config: Dirs,
    picker: RefCell<FolderPicker>,
    builds: RefCell<Option<Option<FirmwareBuild>>>,
    setup: RefCell<Option<SetupSession>>,
    ignored: RefCell<BTreeSet<DeviceId>>,
    started: RefCell<BTreeMap<DeviceId, NewBoard>>,
    tasks: RefCell<BTreeMap<String, FirmwareTask>>,
    attached: RefCell<BTreeSet<String>>,
    watched: RefCell<BTreeSet<String>>,
    workers: RefCell<Vec<tokio::task::JoinHandle<()>>>,
    /// Whether each Device was connected at the last refresh, to read settings when it returns.
    live: RefCell<BTreeMap<DeviceId, bool>>,
    start: Arc<std::sync::Mutex<StartAtSignIn>>,
    updates: RefCell<UpdateChecks>,
    models: Models,
    refreshing: Cell<bool>,
}

impl Drop for Screens {
    fn drop(&mut self) {
        for worker in self.workers.borrow_mut().drain(..) {
            worker.abort();
        }
    }
}

impl Screens {
    pub(crate) fn attach(
        ui: &AppWindow,
        handle: Handle,
        runtime: tokio::runtime::Handle,
        window: Rc<window::State>,
        config: Dirs,
    ) -> Rc<Self> {
        let updates = match AppSettings::load(&config) {
            Ok(settings) => UpdateChecks::Ready {
                on: settings.automatic_update_checks,
                problem: None,
            },
            Err(error) => UpdateChecks::Unreadable(format!(
                "Couldn't read {}, so Open TouchDeck doesn't check for updates. Fix or remove the file, then reopen Open TouchDeck. ({error})",
                config.app_settings().display()
            )),
        };
        let screens = Rc::new(Self {
            ui: ui.as_weak(),
            handle,
            runtime,
            window,
            config,
            picker: RefCell::new(Arc::new(td_companion::photos::folder::pick)),
            builds: RefCell::new(None),
            setup: RefCell::new(None),
            ignored: RefCell::default(),
            started: RefCell::default(),
            tasks: RefCell::default(),
            attached: RefCell::default(),
            watched: RefCell::default(),
            workers: RefCell::default(),
            live: RefCell::default(),
            start: Arc::new(std::sync::Mutex::new(StartAtSignIn::Reading)),
            updates: RefCell::new(updates),
            models: Models::default(),
            refreshing: Cell::new(false),
        });
        SCREENS.with(|s| *s.borrow_mut() = Rc::downgrade(&screens));
        ui.set_boards(ModelRc::from(screens.models.boards.clone()));
        ui.set_ignored_boards(ModelRc::from(screens.models.ignored.clone()));
        ui.set_setup_later(ModelRc::from(screens.models.later.clone()));
        ui.set_settings_devices(ModelRc::from(screens.models.devices.clone()));
        ui.set_paired_apps(ModelRc::from(screens.models.paired.clone()));
        ui.set_after_choices(ModelRc::from(screens.models.after.clone()));
        ui.set_appearance(scheme(screens.window.appearance()));
        screens.wire(ui);
        screens.follow();
        screens.read_start();
        screens.refresh();
        screens
    }

    #[cfg(test)]
    pub(crate) fn set_picker(&self, picker: FolderPicker) {
        *self.picker.borrow_mut() = picker;
    }

    /// Wakes the UI on every Device, New board and Job change.
    fn follow(&self) {
        let handle = self.handle.clone();
        let worker = self.runtime.spawn(async move {
            let mut changes = handle.observe();
            wake();
            loop {
                match changes.recv().await {
                    Ok(
                        Change::DeviceAdded(_)
                        | Change::DeviceUpdated(_)
                        | Change::DeviceRemoved(_)
                        | Change::NewBoardAdded(_)
                        | Change::NewBoardRemoved(_)
                        | Change::JobUpdated(_),
                    ) => wake(),
                    Ok(Change::Stopping) | Err(RecvError::Closed) => return,
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => {
                        changes = handle.observe();
                        wake();
                    }
                }
            }
        });
        self.workers.borrow_mut().push(worker);
    }

    /// Wakes the UI whenever this view changes. Each view is watched once.
    fn watch<T: Send + Sync + 'static>(
        &self,
        key: String,
        mut changes: tokio::sync::watch::Receiver<T>,
    ) {
        if !self.watched.borrow_mut().insert(key) {
            return;
        }
        let worker = self.runtime.spawn(async move {
            while changes.changed().await.is_ok() {
                wake();
            }
        });
        self.workers.borrow_mut().push(worker);
    }

    fn spawn(&self, work: impl Future<Output = ()> + Send + 'static) {
        self.runtime.spawn(work);
    }

    fn task(&self, target: Target) -> FirmwareTask {
        let key = match &target {
            Target::Device(id) => format!("device:{id}"),
            Target::NewBoard(id) => format!("board:{id}"),
        };
        let task = self
            .tasks
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| FirmwareTask::new(self.handle.clone(), target))
            .clone();
        self.watch(key, task.subscribe());
        task
    }

    fn settings(&self, id: &DeviceId) -> SettingsSession {
        let session = self.handle.device_operations().settings_session(id);
        self.watch(format!("settings:{id}"), session.subscribe());
        session
    }

    fn pairings(&self, id: &DeviceId) -> PairingsSession {
        let session = self.handle.device_operations().pairings_session(id);
        self.watch(format!("pairings:{id}"), session.subscribe());
        session
    }

    fn builds(&self) -> Option<FirmwareBuild> {
        let mut builds = self.builds.borrow_mut();
        builds
            .get_or_insert_with(|| self.handle.firmware().builds().ok())
            .clone()
    }

    fn offered(&self, device: &DeviceSummary) -> Option<String> {
        offered(device, self.builds().as_ref())
    }

    fn device(&self, id: &str) -> Option<DeviceSummary> {
        DeviceId::parse(id).and_then(|id| self.handle.devices().get(&id))
    }

    fn selected(&self) -> Option<DeviceSummary> {
        let ui = self.ui.upgrade()?;
        self.device(&ui.get_settings_device())
    }

    /// Follows a Firmware Job that started elsewhere, such as before this window opened.
    fn attach_job(&self, device: &DeviceSummary) {
        let LinkState::Updating { job } = &device.link else {
            return;
        };
        if !self.attached.borrow_mut().insert(job.clone()) {
            return;
        }
        if let Some(job) = self.handle.jobs().get(job) {
            let task = self.task(Target::Device(device.id.clone()));
            self.spawn(async move { task.attach(job).await });
        }
    }

    fn read_start(&self) {
        let (handle, start) = (self.handle.clone(), self.start.clone());
        self.runtime.spawn_blocking(move || {
            *start.lock().unwrap() = match handle.start_at_sign_in() {
                Ok(on) => StartAtSignIn::Ready { on, problem: None },
                Err(_) => StartAtSignIn::Unreadable,
            };
            wake();
        });
    }

    fn set_start(&self, on: bool) {
        let previous = match &*self.start.lock().unwrap() {
            StartAtSignIn::Ready { on, .. } => *on,
            _ => return,
        };
        *self.start.lock().unwrap() = StartAtSignIn::Ready { on, problem: None };
        let (handle, start) = (self.handle.clone(), self.start.clone());
        self.runtime.spawn_blocking(move || {
            if handle.set_start_at_sign_in(on).is_err() {
                let problem = format!(
                    "Couldn't turn {} Start at sign-in. Retry, or restart Open TouchDeck.",
                    if on { "on" } else { "off" }
                );
                *start.lock().unwrap() = StartAtSignIn::Ready {
                    on: previous,
                    problem: Some(problem),
                };
            }
            wake();
        });
        self.refresh();
    }

    fn set_updates(&self, on: bool) {
        let next = match AppSettings::set_automatic_update_checks(&self.config, on) {
            Ok(settings) => UpdateChecks::Ready {
                on: settings.automatic_update_checks,
                problem: None,
            },
            Err(error) => {
                let was = match &*self.updates.borrow() {
                    UpdateChecks::Ready { on, .. } => *on,
                    UpdateChecks::Unreadable(_) => return,
                };
                UpdateChecks::Ready {
                    on: was,
                    problem: Some(format!(
                        "Couldn't save this setting. Retry, or restart Open TouchDeck. ({error})"
                    )),
                }
            }
        };
        *self.updates.borrow_mut() = next;
        self.refresh();
    }

    fn set_appearance(&self, name: &str) {
        let Some(appearance) = Appearance::parse(name) else {
            return;
        };
        self.window.set_appearance(appearance);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_appearance(scheme(appearance));
        }
        self.refresh();
    }

    fn wire(self: &Rc<Self>, ui: &AppWindow) {
        macro_rules! on {
            ($setter:ident, |$screens:ident $(, $arg:ident : $ty:ty)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                ui.$setter(move |$($arg: $ty),*| {
                    if let Some($screens) = weak.upgrade() {
                        $body;
                    }
                });
            }};
        }

        // Setup.
        on!(on_install_board, |s, id: SharedString| s.install_board(&id));
        on!(on_ignore_board, |s, id: SharedString| {
            if let Some(id) = DeviceId::parse(&id) {
                s.ignored.borrow_mut().insert(id);
            }
            s.refresh()
        });
        on!(on_restore_board, |s, id: SharedString| {
            if let Some(id) = DeviceId::parse(&id) {
                s.ignored.borrow_mut().remove(&id);
            }
            s.refresh()
        });
        on!(on_cancel_board, |s, id: SharedString| {
            if let Some(id) = DeviceId::parse(&id) {
                s.task(Target::NewBoard(id.clone())).dismiss();
                s.started.borrow_mut().remove(&id);
            }
            s.refresh()
        });
        on!(on_wizard_name_edited, |s, text: SharedString| {
            if let Some(setup) = s.setup.borrow().clone() {
                setup.set_name(&text);
            }
        });
        on!(on_wizard_next, |s| s.wizard_next());
        on!(on_wizard_back, |s| {
            if let Some(setup) = s.setup.borrow().clone() {
                setup.back();
            }
        });
        on!(on_wizard_close, |s| s.close_setup());
        on!(on_wizard_orientation, |s, value: SharedString| {
            if let (Some(setup), Some(orientation)) =
                (s.setup.borrow().clone(), orientation_of(&value))
            {
                setup.choose_orientation(orientation);
            }
        });
        on!(on_wizard_update, |s| {
            if let Some(setup) = s.setup.borrow().clone() {
                let task = s.task(Target::Device(setup.id().clone()));
                s.spawn(async move { task.start().await });
            }
        });
        on!(on_wizard_skip, |s| {
            if let Some(setup) = s.setup.borrow().clone() {
                setup.next();
            }
        });
        on!(on_start_setup, |s, id: SharedString| s.open_setup(&id));

        // Settings.
        on!(on_set_start, |s, on: bool| s.set_start(on));
        on!(on_set_updates, |s, on: bool| s.set_updates(on));
        on!(on_set_appearance, |s, name: SharedString| s
            .set_appearance(&name));
        on!(on_open_device, |s, id: SharedString| s.open_device(&id));
        on!(on_rename_device, |s, text: SharedString| {
            if let Some(device) = s.selected() {
                let session = s.settings(&device.id);
                let text = text.to_string();
                s.spawn(async move {
                    session.rename(&text).await;
                });
            }
        });
        on!(on_choose_orientation, |s, value: SharedString| {
            if let (Some(device), Some(orientation)) = (s.selected(), orientation_of(&value)) {
                s.settings(&device.id).choose_orientation(orientation);
            }
        });
        on!(on_cancel_orientation, |s| {
            if let Some(device) = s.selected() {
                s.settings(&device.id).cancel_orientation();
            }
        });
        on!(on_confirm_orientation, |s| {
            if let Some(device) = s.selected() {
                let session = s.settings(&device.id);
                s.spawn(async move {
                    session.confirm_orientation().await;
                });
            }
        });
        on!(on_set_photo, |s, on: bool| s.photo(PhotoFramePatch {
            enabled: Some(on),
            ..Default::default()
        }));
        on!(on_set_after, |s, index: i32| {
            if let Some(device) = s.selected() {
                let current = s
                    .settings(&device.id)
                    .view()
                    .shown
                    .and_then(|v| v.photo_frame);
                let choices = start_after_choices(current.map_or(600, |f| f.timeout_s));
                if let Some(seconds) = usize::try_from(index).ok().and_then(|i| choices.get(i)) {
                    s.photo(PhotoFramePatch {
                        timeout_s: Some(*seconds),
                        ..Default::default()
                    });
                }
            }
        });
        on!(on_choose_folder, |s| s.choose_folder());
        on!(on_forget_app, |s, app: SharedString| {
            if let Some(device) = s.selected() {
                let session = s.pairings(&device.id);
                let app = app.to_string();
                s.spawn(async move {
                    session.forget(&app).await;
                });
            }
        });
        on!(on_forget_all, |s| {
            if let Some(device) = s.selected() {
                let session = s.pairings(&device.id);
                s.spawn(async move {
                    session.forget_all().await;
                });
            }
        });
        on!(on_update_firmware, |s| {
            if let Some(device) = s.selected() {
                let task = s.task(Target::Device(device.id.clone()));
                s.spawn(async move { task.start().await });
            }
        });
        on!(on_restart_device, |s| {
            if let Some(device) = s.selected() {
                let session = s.settings(&device.id);
                s.spawn(async move {
                    session.restart().await;
                });
            }
        });
    }

    fn install_board(&self, id: &str) {
        let Some(id) = DeviceId::parse(id) else {
            return;
        };
        let board = self
            .handle
            .devices()
            .new_boards()
            .into_iter()
            .find(|b| b.id == id)
            .or_else(|| self.started.borrow().get(&id).cloned());
        if let Some(board) = board {
            self.started.borrow_mut().insert(id.clone(), board);
        }
        let task = self.task(Target::NewBoard(id));
        self.spawn(async move { task.start().await });
        self.refresh();
    }

    fn open_setup(&self, id: &str) {
        let Some(device) = self.device(id) else {
            return;
        };
        let offered = self.offered(&device).is_some();
        let session = SetupSession::new(self.handle.device_operations().clone(), device, offered);
        self.watch(
            format!("setup:{id}:{}", self.watched.borrow().len()),
            session.subscribe(),
        );
        *self.setup.borrow_mut() = Some(session);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_page(Page::Setup);
        }
        self.refresh();
    }

    fn close_setup(&self) {
        let Some(setup) = self.setup.borrow_mut().take() else {
            return;
        };
        self.handle
            .setup_skips()
            .closed(self.handle.devices().get(setup.id()).as_ref());
        self.refresh();
    }

    fn wizard_next(&self) {
        let Some(setup) = self.setup.borrow().clone() else {
            return;
        };
        let Some(device) = self.handle.devices().get(setup.id()) else {
            return;
        };
        let view = setup.view();
        let updating = self
            .task(Target::Device(device.id.clone()))
            .view()
            .view
            .in_progress()
            || matches!(device.link, LinkState::Updating { .. });
        if !connected(&device) || view.busy.is_some() || updating {
            return;
        }
        match view.flow.at {
            Step::Name => self.spawn(async move { setup.submit_name().await }),
            Step::Orientation => self.spawn(async move { setup.submit_orientation().await }),
            Step::Firmware | Step::Done => setup.next(),
        }
    }

    fn open_device(&self, id: &str) {
        if let Some(device) = self.device(id) {
            let settings = self.settings(&device.id);
            let pairings = self.pairings(&device.id);
            self.spawn(async move {
                settings.load().await;
                pairings.load().await;
            });
        }
        self.refresh();
    }

    fn photo(&self, change: PhotoFramePatch) {
        if let Some(device) = self.selected() {
            let session = self.settings(&device.id);
            self.spawn(async move {
                session.set_photo_frame(change).await;
            });
        }
    }

    fn choose_folder(&self) {
        let Some(device) = self.selected() else {
            return;
        };
        let session = self.settings(&device.id);
        let picker = self.picker.borrow().clone();
        self.spawn(async move {
            let Ok(Ok(Some(folder))) = tokio::task::spawn_blocking(move || picker()).await else {
                return;
            };
            session
                .set_photo_frame(PhotoFramePatch {
                    folder: Some(folder),
                    ..Default::default()
                })
                .await;
        });
    }

    /// Rebuilds Setup and Settings from the Companion and the sessions.
    pub(crate) fn refresh(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        // A refresh never nests: a model change can't call back into it.
        if self.refreshing.replace(true) {
            return;
        }
        let mut devices = self.handle.devices().summaries();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        let boards = self.handle.devices().new_boards();
        for device in &devices {
            self.attach_job(device);
        }
        self.refresh_setup(&ui, &devices, &boards);
        self.refresh_settings(&ui, &devices);
        self.refreshing.set(false);
    }

    fn refresh_setup(&self, ui: &AppWindow, devices: &[DeviceSummary], boards: &[NewBoard]) {
        let skips = self.handle.setup_skips();
        // Setup opens by itself for the first Device that needs it.
        if self.setup.borrow().is_none()
            && let Some(device) = candidate(devices, &skips.skipped())
        {
            let id = device.id.to_string();
            self.refreshing.set(false);
            self.open_setup(&id);
            self.refreshing.set(true);
        }
        let cards = board_cards(
            boards,
            devices,
            &self.ignored.borrow(),
            &self.started.borrow(),
        );
        let rows = cards
            .shown
            .iter()
            .map(|board| {
                let task = self.task(Target::NewBoard(board.id.clone()));
                let view = task.view();
                // The install job itself clears the board's old setup skip.
                let step = board_step(&view.view);
                BoardCard {
                    id: board.id.as_str().into(),
                    port: board.port.as_str().into(),
                    step: match step {
                        BoardStep::Detected => "detected",
                        BoardStep::Waiting => "waiting",
                        BoardStep::Installing => "installing",
                        BoardStep::Done => "done",
                        BoardStep::Declined => "declined",
                        BoardStep::Failed => "failed",
                    }
                    .into(),
                    status: view.view.text().into(),
                    pct: match view.view {
                        JobView::Running { pct: Some(pct), .. } => i32::from(pct),
                        _ => -1,
                    },
                    log: view.log.join("\n").into(),
                }
            })
            .collect();
        sync(&self.models.boards, rows);
        sync(
            &self.models.ignored,
            cards
                .ignored
                .iter()
                .map(|b| SharedString::from(b.id.as_str()))
                .collect(),
        );

        let open = self.setup.borrow().clone();
        let open = open.and_then(|setup| {
            let device = devices.iter().find(|d| &d.id == setup.id());
            if device.is_none() {
                // Unplugged: setup goes, without counting as skipped.
                *self.setup.borrow_mut() = None;
            }
            device.map(|device| (setup, device))
        });
        ui.set_wizard_open(open.is_some());
        if let Some((setup, device)) = &open {
            ui.set_wizard(self.wizard(setup, device));
        }
        let later: Vec<SetupLater> = devices
            .iter()
            .filter(|d| d.setup == SetupState::NeedsSetup)
            .filter(|d| open.as_ref().is_none_or(|(s, _)| s.id() != &d.id))
            .map(|d| SetupLater {
                id: d.id.as_str().into(),
                name: shown_name(d).into(),
            })
            .collect();
        let count = cards.shown.len() + later.len() + usize::from(open.is_some());
        ui.set_setup_count(count as i32);
        sync(&self.models.later, later);
    }

    fn wizard(&self, setup: &SetupSession, device: &DeviceSummary) -> Wizard {
        setup.observe(device);
        let offered = self.offered(device);
        if offered.is_some() && !setup.view().flow.steps.contains(&Step::Firmware) {
            setup.offer_firmware();
        }
        let view = setup.view();
        let task = self.task(Target::Device(device.id.clone())).view();
        let name = called(device);
        let current = device
            .firmware
            .as_ref()
            .map(|f| f.version.clone())
            .unwrap_or_default();
        let live = connected(device);
        let updating = task.view.in_progress() || matches!(device.link, LinkState::Updating { .. });
        let held = view.busy.is_some() || updating;
        let (n, total) = view.flow.count();
        let name_hint = if view.needs_firmware {
            needs_firmware_text(&name)
        } else {
            view.name_error
                .clone()
                .unwrap_or_else(|| "Stored on the Device, so it keeps its name on any PC.".into())
        };
        let (firmware_step, firmware_status, pct) = match &task.view {
            JobView::Idle => ("idle", String::new(), -1),
            JobView::Waiting { text } => ("waiting", text.clone(), -1),
            JobView::Running { text, pct } => ("running", text.clone(), pct.map_or(-1, i32::from)),
            JobView::Done { version, .. } => ("done", format!("Updated to {version}"), 100),
            JobView::Failed { .. } => (
                "failed",
                format!(
                    "Couldn't update. {} went back to {current} and still works.",
                    opening(&name)
                ),
                -1,
            ),
            JobView::Cancelled { text } => ("cancelled", text.clone(), -1),
        };
        let firmware_log = std::iter::once(task.view.text().to_owned())
            .chain(task.log.iter().cloned())
            .collect::<Vec<_>>()
            .join("\n");
        Wizard {
            id: device.id.as_str().into(),
            device_name: if view.name.trim().is_empty() {
                shown_name(device)
            } else {
                view.name.trim().to_owned()
            }
            .into(),
            device_status: home::status(&device.link, device.carrier).0.into(),
            step: match view.flow.at {
                Step::Firmware => "firmware",
                Step::Name => "name",
                Step::Orientation => "orientation",
                Step::Done => "done",
            }
            .into(),
            step_n: n as i32,
            step_total: total as i32,
            first: view.flow.first(),
            name: view.name.as_str().into(),
            name_hint: name_hint.into(),
            name_invalid: view.name_error.is_some() && !view.needs_firmware,
            name_disabled: view.needs_firmware,
            orientation: orientation_name(view.orientation).into(),
            firmware_intro: format!(
                "{} runs Firmware {current}. Firmware {} adds newer Tiles.",
                opening(&name),
                offered.unwrap_or_default()
            )
            .into(),
            firmware_step: firmware_step.into(),
            firmware_status: firmware_status.into(),
            firmware_pct: pct,
            firmware_log: firmware_log.into(),
            keep_plugged: format!("Keep {name} plugged in. It restarts once.").into(),
            busy: view.busy.map_or("", |busy| busy.label()).into(),
            error: view
                .error
                .as_ref()
                .map_or(String::new(), |e| e.text.clone())
                .into(),
            error_detail: view
                .error
                .as_ref()
                .map_or(String::new(), |e| e.detail.clone())
                .into(),
            offline: if !live && !held && view.flow.at != Step::Done {
                format!("Plug in {name} to change its settings.")
            } else {
                String::new()
            }
            .into(),
            live,
            updating,
            done_title: format!("{} is ready", opening(&name)).into(),
            done_text: format!("Tap a Tile on {name} to try it.").into(),
        }
    }

    fn refresh_settings(&self, ui: &AppWindow, devices: &[DeviceSummary]) {
        sync(
            &self.models.devices,
            devices
                .iter()
                .map(|d| DeviceEntry {
                    id: d.id.as_str().into(),
                    name: shown_name(d).into(),
                    status: home::status(&d.link, d.carrier).0.into(),
                })
                .collect(),
        );
        ui.set_pc(self.pc());

        // A Device that came back gets its settings and Pairings read again.
        for device in devices {
            let live = connected(device);
            let was = self.live.borrow_mut().insert(device.id.clone(), live);
            let opened = ui.get_settings_device() == device.id.as_str();
            if opened && live {
                // Read when the page opened before its first read, or when the Device returns.
                let session = self.settings(&device.id);
                if was == Some(false) || session.view().load == td_companion::devices::Load::Idle {
                    self.spawn(async move { session.load().await });
                }
                let pairings = self.pairings(&device.id);
                if offers_pairings(device)
                    && pairings.view().load == td_companion::devices::Load::Idle
                {
                    self.spawn(async move { pairings.load().await });
                }
            }
            if self
                .watched
                .borrow()
                .contains(&format!("pairings:{}", device.id))
            {
                let pairings = self.pairings(&device.id);
                if pairings.observe(device) {
                    self.spawn(async move { pairings.load().await });
                }
            }
        }

        let selected = ui.get_settings_device();
        if selected.is_empty() {
            return;
        }
        let Some(device) = devices.iter().find(|d| selected == d.id.as_str()) else {
            // The Device is gone: back to the list.
            ui.set_settings_device(SharedString::new());
            ui.set_confirming_forget_all(false);
            return;
        };
        let page = self.device_page(ui, device);
        ui.set_device_page(page);
    }

    fn pc(&self) -> PcSettings {
        let (start_on, start_enabled, start_problem) = match &*self.start.lock().unwrap() {
            StartAtSignIn::Reading => (false, false, None),
            StartAtSignIn::Ready { on, problem } => (*on, true, problem.clone()),
            StartAtSignIn::Unreadable => (false, false, Some(START_READ_PROBLEM.to_owned())),
        };
        let (update_on, update_enabled, update_problem) = match &*self.updates.borrow() {
            UpdateChecks::Ready { on, problem } => (*on, true, problem.clone()),
            UpdateChecks::Unreadable(problem) => (false, false, Some(problem.clone())),
        };
        PcSettings {
            start_shown: cfg!(any(windows, feature = "mock", test)),
            update_shown: cfg!(any(windows, feature = "mock", test)),
            start_on,
            start_enabled,
            start_problem: start_problem.unwrap_or_default().into(),
            update_on,
            update_enabled,
            update_problem: update_problem.unwrap_or_default().into(),
            appearance: self.window.appearance().name().into(),
        }
    }

    fn device_page(&self, ui: &AppWindow, device: &DeviceSummary) -> DevicePage {
        let settings = self.settings(&device.id);
        let view = settings.view();
        let task = self.task(Target::Device(device.id.clone())).view();
        let name = shown_name(device);
        let live = connected(device);
        let restarting = view.restart == Restart::Restarting;
        let editable = live && view.editable();
        let shown = view.shown.clone();

        let name_value = shown
            .as_ref()
            .and_then(|s| s.name.clone())
            .or_else(|| device.name.clone())
            .unwrap_or_default();
        let name_hint = view
            .name_problem
            .clone()
            .or_else(|| view.needs.name.then(|| needs_firmware_text(&name)))
            .unwrap_or_default();
        let confirm = view.confirm.map(orientation_name).unwrap_or_default();
        let label = |o: Orientation| match o {
            Orientation::Landscape => "Landscape",
            Orientation::Portrait => "Portrait",
        };

        let frame = shown.as_ref().and_then(|s| s.photo_frame.clone());
        let choices = start_after_choices(frame.as_ref().map_or(600, |f| f.timeout_s));
        sync(
            &self.models.after,
            choices
                .iter()
                .map(|seconds| SharedString::from(duration_label(*seconds)))
                .collect(),
        );
        let after_index = frame
            .as_ref()
            .and_then(|f| choices.iter().position(|c| *c == f.timeout_s))
            .map_or(-1, |i| i as i32);

        // Pairings.
        let offers = offers_pairings(device);
        let usb = live && over_usb(device);
        let pairings = offers.then(|| self.pairings(&device.id).view());
        let apps = pairings
            .as_ref()
            .and_then(|p| p.pairings.as_ref())
            .map(|p| p.pairings.clone())
            .unwrap_or_default();
        if apps.is_empty() {
            ui.set_confirming_forget_all(false);
        }
        sync(
            &self.models.paired,
            apps.iter()
                .map(|app| PairedApp {
                    app: app.app.as_str().into(),
                    name: app.name.as_str().into(),
                    this_app: app.this_app,
                    forgetting: pairings.as_ref().and_then(|p| p.forgetting.as_ref())
                        == Some(&Forgetting::App(app.app.clone())),
                })
                .collect(),
        );

        // Firmware.
        let offered = self.offered(device);
        let version = device.firmware.as_ref().map(|f| f.version.clone());
        let wifi = live && !over_usb(device);
        let busy = task.view.in_progress();
        let done = matches!(task.view, JobView::Done { .. });
        let updating = busy || matches!(device.link, LinkState::Updating { .. });
        let firmware_status = if busy || done {
            task.view.text().to_owned()
        } else if let Some(offered) = &offered {
            format!("{offered} available")
        } else if version.is_some() {
            "Up to date".into()
        } else {
            "Not reported".into()
        };
        let firmware_hint = if matches!(task.view, JobView::Running { .. }) {
            format!("Keep {name} plugged in. Takes about a minute. {name} restarts once.")
        } else if wifi && !busy {
            format!("Plug in {name} with a USB cable to update or restart it.")
        } else {
            String::new()
        };
        let notice = view.notice.clone();

        DevicePage {
            id: device.id.as_str().into(),
            name: name.as_str().into(),
            status: home::status(&device.link, device.carrier).0.into(),
            offline: if !live && !matches!(device.link, LinkState::Updating { .. }) && !restarting
            {
                format!("Plug in {name} to change its settings.")
            } else {
                String::new()
            }
            .into(),
            editable,
            name_value: name_value.into(),
            name_hint: name_hint.into(),
            name_invalid: view.name_problem.is_some(),
            name_disabled: !live || view.needs.name,
            orientation: shown
                .as_ref()
                .map(|s| s.orientation.clone())
                .unwrap_or_default()
                .into(),
            confirm: confirm.into(),
            confirm_title: view
                .confirm
                .map(|o| format!("Restart {name} to switch to {}?", label(o)))
                .unwrap_or_default()
                .into(),
            photo_needs: if view.needs.photo_frame {
                needs_firmware_text(&name)
            } else {
                String::new()
            }
            .into(),
            photo_on: frame.as_ref().is_some_and(|f| f.enabled),
            photo_enabled: editable && frame.is_some(),
            photo_hint: format!(
                "Pictures from this PC when {name} sits idle, or a clock when Open TouchDeck isn't running."
            )
            .into(),
            after_index,
            folder: frame
                .as_ref()
                .and_then(|f| f.folder.clone())
                .unwrap_or_else(|| "No folder chosen".into())
                .into(),
            folder_hint: format!("Nothing is stored on {name}.").into(),
            folder_problem: view.folder_problem.clone().unwrap_or_default().into(),
            can_pick: true,
            pairings_shown: offers,
            pairings_needs: if pairings.as_ref().is_some_and(|p| p.needs) {
                needs_firmware_text(&name)
            } else {
                String::new()
            }
            .into(),
            pairings_count: if usb {
                pairings.as_ref().and_then(|p| p.count()).unwrap_or_default()
            } else {
                String::new()
            }
            .into(),
            pairings_status: pairings
                .as_ref()
                .and_then(|p| p.status(&name, live && !restarting, usb))
                .unwrap_or_else(|| {
                    if pairings.as_ref().is_some_and(|p| p.pairings.is_none()) && usb {
                        "Reading…".into()
                    } else {
                        String::new()
                    }
                })
                .into(),
            pairings_listed: usb && !apps.is_empty(),
            pairings_editable: !restarting
                && pairings.as_ref().is_some_and(|p| p.editable(usb)),
            pairings_error: pairings
                .as_ref()
                .and_then(|p| p.error.clone())
                .unwrap_or_default()
                .into(),
            forgetting_all: pairings.as_ref().and_then(|p| p.forgetting.as_ref())
                == Some(&Forgetting::All),
            firmware_title: version
                .as_ref()
                .map_or("Firmware".into(), |v| format!("Firmware {v}"))
                .into(),
            firmware_status: firmware_status.into(),
            firmware_offer: offered.is_some() && !busy && !done,
            firmware_usb: live && !wifi,
            restart_enabled: live && !wifi && !updating && !restarting,
            firmware_step: match task.view {
                JobView::Idle => "idle",
                JobView::Waiting { .. } => "waiting",
                JobView::Running { .. } => "running",
                JobView::Done { .. } => "done",
                JobView::Failed { .. } => "failed",
                JobView::Cancelled { .. } => "cancelled",
            }
            .into(),
            firmware_pct: match task.view {
                JobView::Running { pct: Some(pct), .. } => i32::from(pct),
                _ => -1,
            },
            firmware_hint: firmware_hint.into(),
            firmware_log: task.log.join("\n").into(),
            notice: notice
                .as_ref()
                .map_or(String::new(), |n| n.text.clone())
                .into(),
            notice_kind: match notice.as_ref().map(|n| n.kind) {
                Some(NoticeKind::Error) => "error",
                Some(NoticeKind::Ok) => "ok",
                _ => "info",
            }
            .into(),
            notice_detail: notice
                .and_then(|n| n.detail)
                .unwrap_or_default()
                .into(),
        }
    }
}

pub(crate) fn scheme(appearance: Appearance) -> slint::language::ColorScheme {
    match appearance {
        Appearance::System => slint::language::ColorScheme::Unknown,
        Appearance::Light => slint::language::ColorScheme::Light,
        Appearance::Dark => slint::language::ColorScheme::Dark,
    }
}
