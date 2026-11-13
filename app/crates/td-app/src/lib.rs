//! Open TouchDeck 2.0's native window: the Companion and a Slint UI in one process (ADR 0011).
//!
//! The program keeps one [`td_companion::companion::Companion`] for its whole life and gives
//! the window a [`Handle`]. [`App::run`] owns the main thread and Slint's event loop until the
//! program ends; the Companion's async work runs on the Tokio runtime beside it. Home follows
//! the Companion's Devices through `snapshot` and `observe`.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let runtime = tokio::runtime::Runtime::new()?;
//! # let data = std::path::PathBuf::new();
//! use td_companion::companion::{Companion, Config};
//! let companion = runtime.block_on(Companion::start(Config::real(data.clone())))?;
//! td_app::init_platform()?;
//! let app = td_app::App::new(td_app::Options { data_dir: data, close_to_tray: true })?;
//! let open = app.opener(); // The tray's "Open" calls `open.open()` from its own thread.
//! app.run(&companion.handle(), runtime.handle())?;
//! runtime.block_on(companion.shutdown());
//! # Ok(())
//! # }
//! ```
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{path::PathBuf, rc::Rc};
use td_companion::companion::Handle;

slint::include_modules!();

mod about;
mod home;
mod look;
pub mod prefs;
mod window;

pub use look::init_platform;
pub use window::Opener;

pub struct Options {
    /// The app data folder, where the window keeps `window.json`.
    pub data_dir: PathBuf,
    /// A tray icon can open the window again, so closing it hides it and the app keeps running.
    /// Without one, closing the window ends the program.
    pub close_to_tray: bool,
}

/// The window, wired to its preferences but not yet shown.
pub struct App {
    ui: AppWindow,
    state: Rc<window::State>,
}

impl App {
    /// Builds the window. Call [`init_platform`] first, except under a test backend.
    pub fn new(options: Options) -> Result<Self, slint::PlatformError> {
        let ui = AppWindow::new()?;
        about::fill(&ui);
        look::fonts(&ui);
        ui.on_open_url(|url| open_url(&url));
        let state = window::attach(
            &ui,
            prefs::Prefs::load(&options.data_dir),
            options.close_to_tray,
        );
        Ok(Self { ui, state })
    }

    /// The handle that opens the window from any thread.
    pub fn opener(&self) -> Opener {
        Opener(self.ui.as_weak())
    }

    /// Shows the window where it was last time and runs the event loop until the program ends:
    /// on Quit, on closing the window without a tray, or when the Companion is asked to stop
    /// (`Handle::request_shutdown`). The caller then shuts the Companion down.
    pub fn run(
        self,
        companion: &Handle,
        runtime: &tokio::runtime::Handle,
    ) -> Result<(), slint::PlatformError> {
        let feed = home::follow(companion.clone(), runtime, self.ui.as_weak());
        let stopping = companion.clone();
        let stop = runtime.spawn(async move {
            stopping.stopping().await;
            let _ = slint::invoke_from_event_loop(|| {
                let _ = slint::quit_event_loop();
            });
        });
        let _look = look::watch(&self.ui);
        let weak = self.ui.as_weak();
        self.ui.on_dark_changed(move |dark| {
            if let Some(ui) = weak.upgrade() {
                look::dark_changed(&ui, dark);
            }
        });

        window::restore(&self.ui, &self.state);
        self.ui.show()?;
        // Slint creates the platform window once the event loop runs, so what needs its handle
        // (the saved monitor check, the app icon, Mica) waits for it here.
        let (weak, state) = (self.ui.as_weak(), self.state.clone());
        slint::spawn_local(async move {
            let Some(ui) = weak.upgrade() else { return };
            // An error means the backend is not winit; the rest still applies.
            let _ = ui.window().winit_window().await;
            window::shown(&ui, &state);
            look::shown(&ui);
        })
        .map_err(|error| slint::PlatformError::Other(error.to_string()))?;

        let result = slint::run_event_loop_until_quit();
        self.state.remember(self.ui.window());
        self.state.save();
        feed.abort();
        stop.abort();
        result
    }
}

/// Opens a web page in the default browser. Only https links leave the app.
fn open_url(url: &str) {
    if !url.starts_with("https://") {
        return;
    }
    if let Err(error) = open::that_detached(url) {
        eprintln!("Open TouchDeck: could not open {url}: {error}");
    }
}

#[cfg(test)]
mod ui_tests;
