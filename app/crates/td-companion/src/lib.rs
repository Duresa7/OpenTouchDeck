pub mod actions;
pub mod api;
#[cfg(any(test, feature = "test-hooks"))]
pub mod bindings;
#[cfg(any(test, feature = "test-hooks"))]
pub mod checkpoints;
mod clock;
pub mod companion;
pub(crate) mod controls;
pub mod devices;
mod dialog_text;
pub mod editor;
mod events;
pub mod features;
pub mod icon_packs;
pub mod integrations;
pub mod jobs;
pub mod layouts;
pub mod link;
pub mod live;
pub mod photos;
mod process;
mod runtime;
mod serve;
pub mod setup;
pub mod startup;
pub mod store;
pub mod tray;
pub use serve::Limits;
#[cfg(any(test, feature = "test-hooks"))]
pub mod testing;
pub use process::run;

/// Wall-clock milliseconds, for display only.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub mod updates;
