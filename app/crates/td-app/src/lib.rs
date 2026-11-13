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
//! let app = td_app::App::new(td_app::Options {
//!     data_dir: data,
//!     config: td_core::app_dirs::Dirs::current(),
//!     close_to_tray: true,
//! })?;
//! let open = app.opener(); // The tray's "Open" calls `open.open()` from its own thread.
//! app.run(&companion.handle(), runtime.handle())?;
//! runtime.block_on(companion.shutdown());
//! # Ok(())
//! # }
//! ```
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{path::PathBuf, rc::Rc, time::Duration};
use td_companion::companion::Handle;

slint::include_modules!();

mod about;
mod home;
mod look;
pub mod prefs;
mod screens;
mod window;

pub use look::init_platform;
pub use window::Opener;

pub struct Options {
    /// The app data folder, where the window keeps `window.json`.
    pub data_dir: PathBuf,
    /// The configuration folder, where `app-settings.json` keeps the update-check switch.
    pub config: td_core::app_dirs::Dirs,
    /// A tray icon can open the window again, so closing it hides it and the app keeps running.
    /// Without one, closing the window ends the program.
    pub close_to_tray: bool,
}

/// The window, wired to its preferences but not yet shown.
pub struct App {
    ui: AppWindow,
    state: Rc<window::State>,
    config: td_core::app_dirs::Dirs,
    screens: std::cell::RefCell<Option<Rc<screens::Screens>>>,
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
        ui.set_appearance(screens::scheme(state.appearance()));
        Ok(Self {
            ui,
            state,
            config: options.config,
            screens: Default::default(),
        })
    }

    /// Wires Setup and Settings to the Companion. `run` does it; tests call it themselves.
    fn connect(
        &self,
        companion: &Handle,
        runtime: &tokio::runtime::Handle,
    ) -> Rc<screens::Screens> {
        self.screens
            .borrow_mut()
            .get_or_insert_with(|| {
                screens::Screens::attach(
                    &self.ui,
                    companion.clone(),
                    runtime.clone(),
                    self.state.clone(),
                    self.config.clone(),
                )
            })
            .clone()
    }

    /// Opens `layout`, `setup`, `settings` or `device:<id>` (that Device's settings), for
    /// screenshots.
    #[cfg(feature = "mock")]
    pub fn open_page(&self, page: &str) {
        match page {
            "layout" => self.ui.set_page(Page::Layout),
            "setup" => self.ui.set_page(Page::Setup),
            "settings" => self.ui.set_page(Page::Settings),
            "about" => self.ui.set_page(Page::About),
            _ => {
                if let Some(id) = page.strip_prefix("device:") {
                    self.ui.set_page(Page::Settings);
                    self.ui.set_settings_device(id.into());
                    self.ui.invoke_open_device(id.into());
                }
            }
        }
    }

    /// Mock builds only: while running, writing a file name into `<dir>/request` saves the
    /// window as drawn to that PNG file in `dir`, for screenshots that don't need the desktop's
    /// own screenshot service.
    #[cfg(feature = "mock")]
    pub fn snapshots(&self, dir: PathBuf) -> slint::Timer {
        let timer = slint::Timer::default();
        let weak = self.ui.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(200),
            move || {
                let request = dir.join("request");
                let Ok(name) = std::fs::read_to_string(&request) else {
                    return;
                };
                let _ = std::fs::remove_file(&request);
                let Some(ui) = weak.upgrade() else { return };
                match ui.window().take_snapshot() {
                    Ok(image) => {
                        let (width, height) = (image.width(), image.height());
                        let path = dir.join(name.trim());
                        if let Err(error) = write_png(&path, image.as_bytes(), width, height) {
                            eprintln!("Open TouchDeck: snapshot failed: {error}");
                        }
                    }
                    Err(error) => eprintln!("Open TouchDeck: snapshot failed: {error}"),
                }
            },
        );
        timer
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
        let screens = self.connect(companion, runtime);
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
        close_editors(&screens, runtime, screens::CLOSE_LIMIT);
        result
    }
}

/// Waits until every editor has saved its queued edits, at most `limit` each, so the caller
/// can then shut the Companion down without losing them. A Device read that never returns
/// can't hold quitting up for longer.
fn close_editors(screens: &screens::Screens, runtime: &tokio::runtime::Handle, limit: Duration) {
    let workers = screens.editor_workers();
    runtime.block_on(async {
        for worker in workers {
            if tokio::time::timeout(limit, worker.close()).await.is_err() {
                eprintln!(
                    "Open TouchDeck: an editor didn't stop in time; its last edits may be unsaved"
                );
            }
        }
    });
}

/// Writes RGBA pixels as a PNG.
#[cfg(feature = "mock")]
fn write_png(path: &std::path::Path, rgba: &[u8], width: u32, height: u32) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgba).map_err(std::io::Error::other)
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
mod screens_tests;
#[cfg(test)]
mod ui_tests;
