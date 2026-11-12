//! Home's Device list. It starts from the Companion's snapshot and follows its changes in order.
use crate::{AppWindow, DeviceRow, Tone};
use slint::{Model, ModelRc, VecModel};
use std::rc::Rc;
use td_companion::{
    companion::{Change, Handle, RecvError},
    devices::{Carrier, DeviceSummary, LinkState},
};

/// What a Device row says about the link, in the frozen React app's words.
pub(crate) fn status(link: &LinkState, carrier: Option<Carrier>) -> (&'static str, Tone) {
    match link {
        LinkState::Connected { .. } => (
            match carrier {
                Some(Carrier::Usb) => "Connected over USB",
                Some(Carrier::Wifi) => "Connected over Wi-Fi",
                None => "Connected",
            },
            Tone::Good,
        ),
        LinkState::Connecting { .. } => ("Connecting…", Tone::Caution),
        LinkState::Updating { .. } => ("Updating…", Tone::Caution),
        LinkState::Busy { .. } => ("In use by another program", Tone::Critical),
        LinkState::Offline
        | LinkState::Lost { .. }
        | LinkState::Disconnected
        | LinkState::Unresponsive { .. } => ("Not connected", Tone::Critical),
    }
}

pub(crate) fn row(device: &DeviceSummary) -> DeviceRow {
    let (status, tone) = status(&device.link, device.carrier);
    let name = device
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Unnamed Device");
    DeviceRow {
        id: device.id.as_str().into(),
        name: name.into(),
        status: status.into(),
        tone,
    }
}

/// The rows of `devices`, in the Companion's order (by id).
pub(crate) fn rows(devices: &[DeviceSummary]) -> Vec<DeviceRow> {
    let mut rows: Vec<_> = devices.iter().map(row).collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

/// Applies one change to the rows, keeping them sorted by id. Other changes leave them alone.
pub(crate) fn apply(model: &VecModel<DeviceRow>, change: &Change) {
    let position =
        |id: &str| (0..model.row_count()).find(|&i| model.row_data(i).is_some_and(|r| r.id == id));
    match change {
        Change::DeviceAdded(added) => upsert(model, row(&added.0)),
        Change::DeviceUpdated(updated) => upsert(model, row(&updated.0)),
        Change::DeviceRemoved(removed) => {
            if let Some(index) = position(removed.id.as_str()) {
                model.remove(index);
            }
        }
        _ => {}
    }
}

fn upsert(model: &VecModel<DeviceRow>, row: DeviceRow) {
    let rows = model.row_count();
    let at = (0..rows).find(|&i| model.row_data(i).is_some_and(|r| r.id >= row.id));
    match at {
        Some(index) if model.row_data(index).is_some_and(|r| r.id == row.id) => {
            model.set_row_data(index, row)
        }
        Some(index) => model.insert(index, row),
        None => model.push(row),
    }
}

/// Shows these Devices, replacing the list.
pub(crate) fn show(ui: &AppWindow, devices: &[DeviceSummary]) {
    ui.set_devices(ModelRc::from(Rc::new(VecModel::from(rows(devices)))));
}

pub(crate) fn update(ui: &AppWindow, change: &Change) {
    let devices = ui.get_devices();
    match devices.as_any().downcast_ref::<VecModel<DeviceRow>>() {
        Some(model) => apply(model, change),
        // Only `show` sets the list, so this is unreachable; rebuilding stays correct anyway.
        None => {
            let model = VecModel::from(devices.iter().collect::<Vec<_>>());
            apply(&model, change);
            ui.set_devices(ModelRc::from(Rc::new(model)));
        }
    }
}

/// Follows the Companion's Devices until the window goes or the Companion stops. It subscribes
/// before reading the snapshot, so no change in between is lost, and starts over after a lag.
pub(crate) fn follow(
    companion: Handle,
    runtime: &tokio::runtime::Handle,
    ui: slint::Weak<AppWindow>,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        loop {
            let mut changes = companion.observe();
            let devices = companion.snapshot().devices;
            if ui
                .upgrade_in_event_loop(move |ui| show(&ui, &devices))
                .is_err()
            {
                return;
            }
            loop {
                match changes.recv().await {
                    Ok(
                        change @ (Change::DeviceAdded(_)
                        | Change::DeviceUpdated(_)
                        | Change::DeviceRemoved(_)),
                    ) => {
                        if ui
                            .upgrade_in_event_loop(move |ui| update(&ui, &change))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(Change::Stopping) | Err(RecvError::Closed) => return,
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => break,
                }
            }
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use td_companion::devices::{
        ControlLayout, ControlLayoutUnavailable, DeviceAdded, DeviceId, DeviceRemoved,
        DeviceUpdated, KeyMirrorState, LostReason, SetupState,
    };

    /// A Device summary with this id, name and link, as the Companion reports it.
    pub(crate) fn device(id: &str, name: Option<&str>, link: LinkState) -> DeviceSummary {
        let carrier = matches!(link, LinkState::Connected { .. }).then_some(Carrier::Usb);
        DeviceSummary {
            id: DeviceId::parse(id).unwrap(),
            rev: 1,
            name: name.map(Into::into),
            setup: SetupState::Ready,
            link,
            carrier,
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

    pub(crate) fn connected() -> LinkState {
        LinkState::Connected {
            port: "mock:a".into(),
            since_ms: 1,
            protocol: 2,
        }
    }

    fn names(model: &VecModel<DeviceRow>) -> Vec<String> {
        model.iter().map(|row| row.name.to_string()).collect()
    }

    #[test]
    fn link_states_read_as_the_react_app_words_them() {
        let lost = LinkState::Lost {
            reason: LostReason::Unplugged,
            detail: String::new(),
            at_ms: 1,
        };
        let cases = [
            (
                connected(),
                Some(Carrier::Usb),
                "Connected over USB",
                Tone::Good,
            ),
            (
                connected(),
                Some(Carrier::Wifi),
                "Connected over Wi-Fi",
                Tone::Good,
            ),
            (connected(), None, "Connected", Tone::Good),
            (
                LinkState::Connecting { port: "p".into() },
                None,
                "Connecting…",
                Tone::Caution,
            ),
            (
                LinkState::Updating { job: "j".into() },
                None,
                "Updating…",
                Tone::Caution,
            ),
            (
                LinkState::Busy {
                    port: "p".into(),
                    detail: String::new(),
                },
                None,
                "In use by another program",
                Tone::Critical,
            ),
            (LinkState::Offline, None, "Not connected", Tone::Critical),
            (lost, None, "Not connected", Tone::Critical),
            (
                LinkState::Disconnected,
                None,
                "Not connected",
                Tone::Critical,
            ),
            (
                LinkState::Unresponsive { port: "p".into() },
                None,
                "Not connected",
                Tone::Critical,
            ),
        ];
        for (link, carrier, words, tone) in cases {
            assert_eq!(status(&link, carrier), (words, tone), "{link:?}");
        }
    }

    #[test]
    fn a_device_without_a_name_is_unnamed() {
        for name in [None, Some(""), Some("   ")] {
            assert_eq!(
                row(&device("a", name, LinkState::Offline)).name,
                "Unnamed Device"
            );
        }
        assert_eq!(
            row(&device("a", Some(" Desk "), LinkState::Offline)).name,
            "Desk"
        );
    }

    #[test]
    fn changes_keep_the_rows_in_id_order() {
        let model = VecModel::from(rows(&[
            device("c", Some("C"), LinkState::Offline),
            device("a", Some("A"), connected()),
        ]));
        assert_eq!(names(&model), ["A", "C"]);

        apply(
            &model,
            &Change::DeviceAdded(DeviceAdded(device("b", Some("B"), connected()))),
        );
        apply(
            &model,
            &Change::DeviceAdded(DeviceAdded(device("d", Some("D"), connected()))),
        );
        assert_eq!(names(&model), ["A", "B", "C", "D"]);

        apply(
            &model,
            &Change::DeviceUpdated(DeviceUpdated(device("c", Some("C"), connected()))),
        );
        assert_eq!(model.row_data(2).unwrap().status, "Connected over USB");
        assert_eq!(model.row_count(), 4);

        apply(
            &model,
            &Change::DeviceRemoved(DeviceRemoved {
                id: DeviceId::parse("a").unwrap(),
            }),
        );
        assert_eq!(names(&model), ["B", "C", "D"]);
    }

    #[test]
    fn an_update_for_an_unknown_device_adds_it_and_other_changes_are_ignored() {
        let model = VecModel::from(Vec::<DeviceRow>::new());
        apply(
            &model,
            &Change::DeviceUpdated(DeviceUpdated(device("a", Some("A"), connected()))),
        );
        apply(&model, &Change::Stopping);
        apply(
            &model,
            &Change::DeviceRemoved(DeviceRemoved {
                id: DeviceId::parse("zz").unwrap(),
            }),
        );
        assert_eq!(names(&model), ["A"]);
    }
}
