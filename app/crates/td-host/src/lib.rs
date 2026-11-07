//! What the app does on the PC for the deck: type its mirrored keys, run its tile actions, and
//! make its icons.

pub mod actions;
pub mod approval;
pub mod catalog;
pub mod gif_normalize;
pub mod icon_pack;
pub mod iconify;
pub mod icons;
#[cfg(target_os = "linux")]
mod icons_linux;
#[cfg(windows)]
mod icons_windows;
pub mod input;
#[cfg(target_os = "linux")]
mod input_linux;
#[cfg(windows)]
mod input_windows;
pub mod keymap;
pub mod packs;
pub mod sources;
mod tarball;

pub use approval::{Approvals, CommandSpec, Grant};
pub use input::{FakeSink, InputSink, KeyEvent, Mirror, Mirrored, Outcome};

/// Brings a window to the foreground without sending any key. Returns whether it is in front.
#[cfg(windows)]
pub fn bring_to_front(hwnd: isize) -> bool {
    // SAFETY: Win32 checks the handle and fails the calls on one that names no window.
    unsafe { input_windows::bring_to_front(hwnd as *mut std::ffi::c_void) }
}

pub mod integrations;
