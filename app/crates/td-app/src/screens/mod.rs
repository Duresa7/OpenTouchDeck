//! Setup and Settings, fed from the Companion. Every rule they follow lives in td-companion
//! (`setup`, the Device settings and Pairings sessions, Firmware tasks) or td-core (the update
//! setting); this module only turns those views into the window's properties and sends the
//! window's callbacks back as typed operations.
//!
//! The Companion's work runs on Tokio. Each session publishes its view on a `watch` channel; a
//! small task per channel wakes the UI thread, which then rebuilds the window's properties from
//! the current views in [`Screens::refresh`]. The headless tests call `refresh` themselves.
use crate::{AppWindow, BoardCard, DeviceEntry, PairedApp, SetupLater, home, window};
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
    devices::{DeviceId, DeviceSummary, LinkState, NewBoard},
    features::firmware::{FirmwareBuild, FirmwareTask, Target, offered},
    setup::SetupSession,
};
use td_core::{app_dirs::Dirs, settings::AppSettings};

macro_rules! on {
    ($owner:expr, $ui:ident, $setter:ident, |$screens:ident $(, $arg:ident : $ty:ty)*| $body:expr) => {{
        let weak = Rc::downgrade($owner);
        $ui.$setter(move |$($arg: $ty),*| {
            if let Some($screens) = weak.upgrade() {
                $body;
            }
        });
    }};
}

mod settings;
mod setup;

pub(crate) use settings::{FolderPicker, scheme};
use settings::{StartAtSignIn, UpdateChecks};

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
        screens.wire_setup(ui);
        screens.wire_settings(ui);
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
}
