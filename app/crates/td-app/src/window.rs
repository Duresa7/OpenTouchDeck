//! The window's life: where it opens, what closing it does, and the handle that opens it again.
use crate::{
    AppWindow,
    prefs::{Placement, Prefs},
};
use slint::{
    CloseRequestResponse, ComponentHandle, LogicalSize, PhysicalPosition, Timer, TimerMode,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

/// What closing the window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Close {
    /// Nothing could bring the window back, so the program ends.
    Quit,
    /// The first close with a tray: explain that the app keeps running.
    Notice,
    /// The tray keeps the app running; the window hides.
    Hide,
}

pub(crate) fn close_action(close_to_tray: bool, notice_shown: bool) -> Close {
    match (close_to_tray, notice_shown) {
        (false, _) => Close::Quit,
        (true, false) => Close::Notice,
        (true, true) => Close::Hide,
    }
}

pub(crate) struct State {
    prefs: RefCell<Prefs>,
    close_to_tray: bool,
    /// Saves a moment after the window stops moving, so a killed process loses little.
    save_soon: Timer,
    quit_requested: Cell<bool>,
}
impl State {
    /// Records where the window is. While it's maximised the last normal place stays, so
    /// restoring it later returns there; a minimised or hidden window records nothing.
    pub(crate) fn remember(&self, window: &slint::Window) {
        if window.is_minimized() || !window.is_visible() {
            return;
        }
        let maximized = window.is_maximized();
        let mut prefs = self.prefs.borrow_mut();
        let placement = &mut prefs.value.placement;
        match placement {
            Some(known) if maximized => known.maximized = true,
            _ => {
                let size = window.size().to_logical(window.scale_factor());
                let position = window.position();
                *placement = Some(Placement {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                    maximized,
                });
            }
        }
    }

    pub(crate) fn save(&self) {
        let prefs = self.prefs.borrow();
        if let Err(error) = prefs.save() {
            eprintln!(
                "Open TouchDeck: could not save {}: {error}",
                prefs.path().display()
            );
        }
    }

    pub(crate) fn appearance(&self) -> crate::prefs::Appearance {
        self.prefs.borrow().value.appearance
    }

    pub(crate) fn set_appearance(&self, appearance: crate::prefs::Appearance) {
        self.prefs.borrow_mut().value.appearance = appearance;
        self.save();
    }

    fn notice_seen(&self) {
        self.prefs.borrow_mut().value.close_notice_shown = true;
        self.save();
    }

    fn quit(&self) {
        self.quit_requested.set(true);
        // Fails only when no event loop runs, as in the headless tests.
        let _ = slint::quit_event_loop();
    }

    #[cfg(test)]
    pub(crate) fn quit_requested(&self) -> bool {
        self.quit_requested.get()
    }
}

thread_local! {
    /// The window's state, for `reveal`. An `Opener` crosses threads, so it can't hold the `Rc`.
    static STATE: RefCell<Weak<State>> = const { RefCell::new(Weak::new()) };
}

/// Wires closing and the close notice.
pub(crate) fn attach(ui: &AppWindow, prefs: Prefs, close_to_tray: bool) -> Rc<State> {
    let state = Rc::new(State {
        prefs: RefCell::new(prefs),
        close_to_tray,
        save_soon: Timer::default(),
        quit_requested: Cell::new(false),
    });
    STATE.with(|current| *current.borrow_mut() = Rc::downgrade(&state));

    let weak = ui.as_weak();
    let on_close = state.clone();
    ui.window().on_close_requested(move || {
        let Some(ui) = weak.upgrade() else {
            return CloseRequestResponse::HideWindow;
        };
        on_close.remember(ui.window());
        let shown = on_close.prefs.borrow().value.close_notice_shown;
        match close_action(on_close.close_to_tray, shown) {
            Close::Quit => {
                on_close.save();
                on_close.quit();
                CloseRequestResponse::HideWindow
            }
            Close::Notice => {
                ui.set_close_notice_open(true);
                CloseRequestResponse::KeepWindowShown
            }
            Close::Hide => {
                on_close.save();
                CloseRequestResponse::HideWindow
            }
        }
    });

    let weak = ui.as_weak();
    let keep = state.clone();
    ui.on_keep_running(move || {
        keep.notice_seen();
        if let Some(ui) = weak.upgrade() {
            ui.set_close_notice_open(false);
            let _ = ui.hide();
        }
    });

    let weak = ui.as_weak();
    let quit = state.clone();
    ui.on_quit(move || {
        quit.notice_seen();
        if let Some(ui) = weak.upgrade() {
            ui.set_close_notice_open(false);
            let _ = ui.hide();
        }
        quit.quit();
    });

    state
}

/// Puts the window where it was last time. Call before the first `show`.
pub(crate) fn restore(ui: &AppWindow, state: &State) {
    if let Some(placement) = state.prefs.borrow().value.placement {
        ui.window()
            .set_size(LogicalSize::new(placement.width, placement.height));
        ui.window()
            .set_position(PhysicalPosition::new(placement.x, placement.y));
    }
}

/// After the first `show`: maximise if it was, bring it back if its monitor is gone, and start
/// following moves and resizes.
pub(crate) fn shown(ui: &AppWindow, state: &Rc<State>) {
    let window = ui.window();
    if state
        .prefs
        .borrow()
        .value
        .placement
        .is_some_and(|placement| placement.maximized)
    {
        window.set_maximized(true);
    }
    keep_on_screen(window);

    let weak = Rc::downgrade(state);
    window.on_winit_window_event(move |window, event| {
        if let (
            Some(state),
            winit::event::WindowEvent::Moved(_) | winit::event::WindowEvent::Resized(_),
        ) = (weak.upgrade(), event)
        {
            state.remember(window);
            let saver = Rc::downgrade(&state);
            state.save_soon.start(
                TimerMode::SingleShot,
                Duration::from_millis(750),
                move || {
                    if let Some(state) = saver.upgrade() {
                        state.save();
                    }
                },
            );
        }
        EventResult::Propagate
    });
}

/// A rectangle in physical screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    fn overlap(self, other: Rect) -> (i64, i64) {
        let span = |a: i32, a_len: u32, b: i32, b_len: u32| {
            let start = i64::from(a.max(b));
            let end = (i64::from(a) + i64::from(a_len)).min(i64::from(b) + i64::from(b_len));
            (end - start).max(0)
        };
        (
            span(self.x, self.width, other.x, other.width),
            span(self.y, self.height, other.y, other.height),
        )
    }
}

/// The window can still be dragged: enough of its title bar is on some monitor.
pub(crate) fn reachable(window: Rect, monitors: &[Rect]) -> bool {
    let title_bar = Rect {
        height: window.height.min(32),
        ..window
    };
    let need_width = i64::from(window.width.min(64));
    let need_height = i64::from(title_bar.height.min(16));
    monitors.iter().any(|monitor| {
        let (width, height) = title_bar.overlap(*monitor);
        width >= need_width && height >= need_height && need_height > 0
    })
}

/// The top-left corner that centres `size` on `monitor`.
pub(crate) fn centred(monitor: Rect, width: u32, height: u32) -> (i32, i32) {
    let offset = |outer: u32, inner: u32| (i64::from(outer) - i64::from(inner)).max(0) / 2;
    (
        (i64::from(monitor.x) + offset(monitor.width, width)) as i32,
        (i64::from(monitor.y) + offset(monitor.height, height)) as i32,
    )
}

/// A window saved on a monitor that has since gone opens centred on the primary one.
/// Wayland doesn't let a window know or choose its position, so there it's left alone.
fn keep_on_screen(window: &slint::Window) {
    window.with_winit_window(|window| {
        let Ok(position) = window.outer_position() else {
            return;
        };
        let size = window.outer_size();
        let rect = |position: winit::dpi::PhysicalPosition<i32>,
                    size: winit::dpi::PhysicalSize<u32>| Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        };
        let monitors: Vec<_> = window
            .available_monitors()
            .map(|monitor| rect(monitor.position(), monitor.size()))
            .collect();
        if monitors.is_empty() || reachable(rect(position, size), &monitors) {
            return;
        }
        let primary = window
            .primary_monitor()
            .map(|monitor| rect(monitor.position(), monitor.size()))
            .unwrap_or(monitors[0]);
        let (x, y) = centred(primary, size.width, size.height);
        window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
    });
}

/// Opens the window from anywhere, such as the tray's thread.
#[derive(Clone)]
pub struct Opener(pub(crate) slint::Weak<AppWindow>);
impl Opener {
    /// Shows the window, restores it if minimised, and brings it to the front. It does nothing
    /// once the program is ending.
    pub fn open(&self) {
        let _ = self.0.upgrade_in_event_loop(|ui| reveal(&ui));
    }
}

pub(crate) fn reveal(ui: &AppWindow) {
    let was_hidden = !ui.window().is_visible();
    if let Err(error) = ui.show() {
        eprintln!("Open TouchDeck: could not show the window: {error}");
        return;
    }
    let window = ui.window();
    // Some window managers place a window afresh each time it's shown (KWin with X11 does);
    // a window coming back from the tray returns to where it was.
    let placement = STATE.with(|state| {
        state
            .borrow()
            .upgrade()
            .and_then(|state| state.prefs.borrow().value.placement)
    });
    if let Some(placement) = placement.filter(|placement| was_hidden && !placement.maximized) {
        window.set_position(PhysicalPosition::new(placement.x, placement.y));
        // Its monitor may have gone while the window was hidden.
        keep_on_screen(window);
    }
    if window.is_minimized() {
        window.set_minimized(false);
    }
    window.with_winit_window(|window| window.focus_window());
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    const LEFT: Rect = Rect {
        x: -2560,
        y: -200,
        width: 2560,
        height: 1440,
    };

    fn window(x: i32, y: i32) -> Rect {
        Rect {
            x,
            y,
            width: 960,
            height: 640,
        }
    }

    #[test]
    fn closing_quits_without_a_tray_and_explains_once_with_one() {
        assert_eq!(close_action(false, false), Close::Quit);
        assert_eq!(close_action(false, true), Close::Quit);
        assert_eq!(close_action(true, false), Close::Notice);
        assert_eq!(close_action(true, true), Close::Hide);
    }

    #[test]
    fn a_window_on_a_connected_monitor_is_reachable() {
        assert!(reachable(window(100, 100), &[MAIN]));
        assert!(reachable(window(-1500, 300), &[MAIN, LEFT]));
        // Mostly off the right edge, but 200 px of title bar remain.
        assert!(reachable(window(1720, 400), &[MAIN]));
        // Windows puts a maximised window's frame a few pixels off screen.
        assert!(reachable(
            Rect {
                x: -8,
                y: -8,
                width: 1936,
                height: 1096
            },
            &[MAIN]
        ));
    }

    #[test]
    fn a_window_whose_monitor_is_gone_or_whose_title_bar_is_hidden_is_not() {
        assert!(!reachable(window(-1500, 300), &[MAIN]));
        assert!(!reachable(window(3000, 100), &[MAIN]));
        // The title bar is above the top of the screen.
        assert!(!reachable(window(100, -600), &[MAIN]));
        // Only a 20 px sliver at the right edge.
        assert!(!reachable(window(1900, 100), &[MAIN]));
        assert!(!reachable(window(100, 100), &[]));
    }

    #[test]
    fn centring_puts_the_window_in_the_middle_of_the_monitor() {
        assert_eq!(centred(MAIN, 960, 640), (480, 220));
        assert_eq!(centred(LEFT, 960, 640), (-2560 + 800, -200 + 400));
        assert_eq!(
            centred(MAIN, 4000, 3000),
            (0, 0),
            "a larger window starts at the corner"
        );
    }
}
