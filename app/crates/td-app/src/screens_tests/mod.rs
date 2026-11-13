//! Setup and Settings, headless, against the in-process Companion and mock Devices. They act the
//! way a screen reader does: find controls by role and label and invoke them.
use crate::{
    App, BoardCard, DevicePage, Options, Page, PairedApp, Theme, Wizard, look, prefs::Prefs,
    screens::Screens,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use slint::{
    ComponentHandle, LogicalSize, SharedString,
    platform::{Key, WindowEvent},
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
use td_companion::{
    companion::{Companion, Config},
    devices::{BoardKind, DeviceId, LinkState, MockPorts, SetupState},
    features::firmware::ConsentAnswer,
};

struct Rig {
    runtime: tokio::runtime::Runtime,
    companion: Option<Companion>,
    ports: MockPorts,
    app: App,
    screens: Rc<Screens>,
    dir: tempfile::TempDir,
}

impl Rig {
    /// A Companion on mock ports, the window wired to it and shown at `width` × 2000 px so the
    /// whole page can be searched.
    fn start(width: f32) -> Self {
        Self::start_with(width, |_| {})
    }

    /// As [`Rig::start`], with `prepare` run on the Companion before the window connects.
    fn start_with(width: f32, prepare: impl FnOnce(&td_companion::companion::Handle)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let mut config = Config::mock(dir.path().join("data"));
        config.devices = td_companion::devices::Config::mock(None);
        let companion = runtime.block_on(Companion::start(config)).unwrap();
        let handle = companion.handle();
        let ports = handle.devices().mock().unwrap().0.clone();
        handle.firmware().script(vec![ConsentAnswer::Accept; 16]);
        prepare(&handle);
        crate::ui_tests::init();
        let app = App::new(Options {
            data_dir: dir.path().join("data"),
            config: td_core::app_dirs::Dirs {
                config: dir.path().join("config"),
            },
            close_to_tray: false,
        })
        .unwrap();
        app.ui.window().set_size(LogicalSize::new(width, 2000.0));
        app.ui.show().unwrap();
        let screens = app.connect(&handle, runtime.handle());
        Self {
            runtime,
            companion: Some(companion),
            ports,
            app,
            screens,
            dir,
        }
    }

    fn handle(&self) -> td_companion::companion::Handle {
        self.companion.as_ref().unwrap().handle()
    }

    /// Refreshes the window until `done` holds, as the UI thread would on each change.
    fn until(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let started = Instant::now();
        loop {
            self.screens.refresh();
            settle(&self.app);
            if done(self) {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn plug(&self, serial: &str, kind: BoardKind) -> DeviceId {
        self.ports.plug(serial, kind);
        let id = DeviceId::parse(serial).unwrap();
        let handle = self.handle();
        self.until("the Device to connect", |_| {
            handle
                .devices()
                .get(&id)
                .is_some_and(|d| matches!(d.link, LinkState::Connected { .. }))
        });
        id
    }

    fn deck(&self, serial: &str) -> td_link::MockDeck {
        self.ports.deck(serial).unwrap()
    }

    fn labelled(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.app.ui, label).collect()
    }

    fn has(&self, label: &str) -> bool {
        !self.labelled(label).is_empty()
    }

    fn one(&self, label: &str, role: AccessibleRole) -> ElementHandle {
        let found: Vec<_> = self
            .labelled(label)
            .into_iter()
            .filter(|e| e.accessible_role() == Some(role))
            .collect();
        assert_eq!(found.len(), 1, "{role:?} elements labelled {label:?}");
        found.into_iter().next().unwrap()
    }

    fn press(&self, label: &str) {
        self.one(label, AccessibleRole::Button)
            .invoke_accessible_default_action();
    }

    fn text_containing(&self, needle: &str) -> bool {
        let needle = needle.to_owned();
        !self
            .app
            .ui
            .root_element()
            .query_descendants()
            .match_predicate(move |e| e.accessible_label().is_some_and(|l| l.contains(&needle)))
            .find_all()
            .is_empty()
    }

    /// Types into a text field and presses Enter, as a person would.
    fn type_and_enter(&self, label: &str, text: &str) {
        let field = self.one(label, AccessibleRole::TextInput);
        field.set_accessible_value(text);
        field.mock_single_click(slint::platform::PointerEventButton::Left);
        let window = self.app.ui.window();
        window.dispatch_event(WindowEvent::KeyPressed {
            text: Key::Return.into(),
        });
        window.dispatch_event(WindowEvent::KeyReleased {
            text: Key::Return.into(),
        });
    }

    fn open_device_settings(&self, name: &str) {
        self.app.ui.set_page(Page::Settings);
        self.until("the Device in Settings", |r| r.has(name));
        self.one(name, AccessibleRole::Button)
            .invoke_accessible_default_action();
        self.until("its settings to load", |r| {
            r.app.ui.get_device_page().editable
        });
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(companion) = self.companion.take() {
            self.runtime.block_on(companion.shutdown());
        }
    }
}

fn settle(app: &App) {
    app.ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(-1.0, -1.0),
    });
}

mod firmware;
mod integrations;
mod settings;
mod setup;

/// Every control, measured with real fonts, ends inside the window.
fn assert_fits(rig: &Rig, width: f32, page: &str) {
    for role in [
        AccessibleRole::Button,
        AccessibleRole::Tab,
        AccessibleRole::Switch,
        AccessibleRole::RadioButton,
        AccessibleRole::TextInput,
        AccessibleRole::Combobox,
    ] {
        for element in rig
            .app
            .ui
            .root_element()
            .query_descendants()
            .match_accessible_role(role)
            .find_all()
        {
            let left = element.absolute_position().x;
            let right = left + element.size().width;
            let label = element.accessible_label().unwrap_or_default();
            assert!(
                left >= 0.0 && right <= width + 0.5,
                "{page}: {label:?} spans {left}..{right} in a {width} px window"
            );
        }
    }
}

#[test]
fn at_the_largest_text_size_setup_and_settings_fit_the_narrowest_window() {
    let rig = Rig::start(500.0);
    // Tall enough for the whole page: the search skips what is outside the window.
    rig.app
        .ui
        .window()
        .set_size(LogicalSize::new(500.0, 12000.0));
    look::apply(
        &rig.app.ui,
        &look::Look {
            text_scale: 2.25,
            ..Default::default()
        },
    );
    rig.ports
        .board("mock0001", BoardKind::DeviceRadio)
        .add_net_pairing([0x21; 16], "Laptop");
    rig.plug("mock0001", BoardKind::DeviceRadio);
    rig.ports.plug("mock0009", BoardKind::Silent);
    rig.app.ui.set_page(Page::Setup);
    rig.until("the New board card", |r| r.has("Install Firmware"));
    assert_fits(&rig, 500.0, "Setup");

    rig.open_device_settings("Mock Device mock0001");
    rig.until("the paired apps", |r| r.has("Forget Laptop"));
    // With the photo frame on, its Start after and Folder rows show too.
    rig.one("Photo frame", AccessibleRole::Switch)
        .invoke_accessible_default_action();
    rig.until("the photo frame rows", |r| r.has("Choose folder"));
    assert_fits(&rig, 500.0, "Device settings");
    rig.app.ui.set_settings_device(SharedString::new());
    settle(&rig.app);
    assert_fits(&rig, 500.0, "Settings");
}
