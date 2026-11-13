use super::{Screens, connected, needs_firmware_text, shown_name, sync, wake};
use crate::{AppWindow, DeviceEntry, DevicePage, PairedApp, PcSettings, home, prefs::Appearance};
use slint::SharedString;
use std::{rc::Rc, sync::Arc};
use td_companion::{
    devices::{
        DeviceId, DeviceSummary, Forgetting, LinkState, NoticeKind, PairingsSession,
        PhotoFramePatch, Restart, SettingsSession, duration_label, offers_pairings,
        orientation_name, orientation_of, over_usb, start_after_choices,
    },
    features::firmware::{JobView, Target},
};
use td_core::{Orientation, settings::AppSettings};

/// Asks the person for a folder. Blocking; it runs off the UI thread.
pub(crate) type FolderPicker = Arc<dyn Fn() -> Result<Option<String>, String> + Send + Sync>;

/// Whether Start at sign-in can be read and what it is.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum StartAtSignIn {
    Reading,
    Ready { on: bool, problem: Option<String> },
    Unreadable,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum UpdateChecks {
    Ready { on: bool, problem: Option<String> },
    Unreadable(String),
}

const START_READ_PROBLEM: &str =
    "Couldn't read Start at sign-in. Restart Open TouchDeck, then retry.";

impl Screens {
    pub(super) fn wire_settings(self: &Rc<Self>, ui: &AppWindow) {
        // Settings.
        on!(self, ui, on_set_start, |s, on: bool| s.set_start(on));
        on!(self, ui, on_set_updates, |s, on: bool| s.set_updates(on));
        on!(self, ui, on_set_appearance, |s, name: SharedString| s
            .set_appearance(&name));
        on!(self, ui, on_open_device, |s, id: SharedString| s
            .open_device(&id));
        on!(self, ui, on_rename_device, |s, text: SharedString| {
            if let Some(device) = s.selected() {
                let session = s.settings(&device.id);
                let text = text.to_string();
                s.spawn(async move {
                    session.rename(&text).await;
                });
            }
        });
        on!(self, ui, on_choose_orientation, |s, value: SharedString| {
            if let (Some(device), Some(orientation)) = (s.selected(), orientation_of(&value)) {
                s.settings(&device.id).choose_orientation(orientation);
            }
        });
        on!(self, ui, on_cancel_orientation, |s| {
            if let Some(device) = s.selected() {
                s.settings(&device.id).cancel_orientation();
            }
        });
        on!(self, ui, on_confirm_orientation, |s| {
            if let Some(device) = s.selected() {
                let session = s.settings(&device.id);
                s.spawn(async move {
                    session.confirm_orientation().await;
                });
            }
        });
        on!(self, ui, on_set_photo, |s, on: bool| s.photo(
            PhotoFramePatch {
                enabled: Some(on),
                ..Default::default()
            }
        ));
        on!(self, ui, on_set_after, |s, index: i32| {
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
        on!(self, ui, on_choose_folder, |s| s.choose_folder());
        on!(self, ui, on_forget_app, |s, app: SharedString| {
            if let Some(device) = s.selected() {
                let session = s.pairings(&device.id);
                let app = app.to_string();
                s.spawn(async move {
                    session.forget(&app).await;
                });
            }
        });
        on!(self, ui, on_forget_all, |s| {
            if let Some(device) = s.selected() {
                let session = s.pairings(&device.id);
                s.spawn(async move {
                    session.forget_all().await;
                });
            }
        });
        on!(self, ui, on_update_firmware, |s| {
            if let Some(device) = s.selected() {
                let task = s.task(Target::Device(device.id.clone()));
                s.spawn(async move { task.start().await });
            }
        });
        on!(self, ui, on_restart_device, |s| {
            if let Some(device) = s.selected() {
                let session = s.settings(&device.id);
                s.spawn(async move {
                    session.restart().await;
                });
            }
        });
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

    fn selected(&self) -> Option<DeviceSummary> {
        let ui = self.ui.upgrade()?;
        self.device(&ui.get_settings_device())
    }

    pub(super) fn read_start(&self) {
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

    pub(super) fn refresh_settings(&self, ui: &AppWindow, devices: &[DeviceSummary]) {
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
