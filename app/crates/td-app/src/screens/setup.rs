use super::{Screens, called, connected, needs_firmware_text, opening, shown_name, sync};
use crate::{AppWindow, BoardCard, Page, SetupLater, Wizard, home};
use slint::SharedString;
use std::rc::Rc;
use td_companion::{
    devices::{
        DeviceId, DeviceSummary, LinkState, NewBoard, SetupState, orientation_name, orientation_of,
    },
    features::firmware::{BoardStep, JobView, Target, board_step},
    setup::{SetupSession, Step, board_cards, candidate},
};

impl Screens {
    pub(super) fn wire_setup(self: &Rc<Self>, ui: &AppWindow) {
        // Setup.
        on!(self, ui, on_install_board, |s, id: SharedString| s
            .install_board(&id));
        on!(self, ui, on_ignore_board, |s, id: SharedString| {
            if let Some(id) = DeviceId::parse(&id) {
                s.ignored.borrow_mut().insert(id);
            }
            s.refresh()
        });
        on!(self, ui, on_restore_board, |s, id: SharedString| {
            if let Some(id) = DeviceId::parse(&id) {
                s.ignored.borrow_mut().remove(&id);
            }
            s.refresh()
        });
        on!(self, ui, on_cancel_board, |s, id: SharedString| {
            if let Some(id) = DeviceId::parse(&id) {
                s.task(Target::NewBoard(id.clone())).dismiss();
                s.started.borrow_mut().remove(&id);
            }
            s.refresh()
        });
        on!(self, ui, on_wizard_name_edited, |s, text: SharedString| {
            if let Some(setup) = s.setup.borrow().clone() {
                setup.set_name(&text);
            }
        });
        on!(self, ui, on_wizard_next, |s| s.wizard_next());
        on!(self, ui, on_wizard_back, |s| {
            if let Some(setup) = s.setup.borrow().clone() {
                setup.back();
            }
        });
        on!(self, ui, on_wizard_close, |s| s.close_setup());
        on!(self, ui, on_wizard_orientation, |s, value: SharedString| {
            if let (Some(setup), Some(orientation)) =
                (s.setup.borrow().clone(), orientation_of(&value))
            {
                setup.choose_orientation(orientation);
            }
        });
        on!(self, ui, on_wizard_update, |s| {
            if let Some(setup) = s.setup.borrow().clone() {
                let task = s.task(Target::Device(setup.id().clone()));
                s.spawn(async move { task.start().await });
            }
        });
        on!(self, ui, on_wizard_skip, |s| {
            if let Some(setup) = s.setup.borrow().clone() {
                setup.next();
            }
        });
        on!(self, ui, on_start_setup, |s, id: SharedString| s
            .open_setup(&id));
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

    pub(super) fn refresh_setup(
        &self,
        ui: &AppWindow,
        devices: &[DeviceSummary],
        boards: &[NewBoard],
    ) {
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
            // Setup only updates from the bundled Firmware, so it never opens a file dialog.
            JobView::Choosing { text } | JobView::Waiting { text } => ("waiting", text.clone(), -1),
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
}
