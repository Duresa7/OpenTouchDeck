//! The notification-area icon: left-click, Enter or Space opens the app's window; the menu has
//! Open, Start at sign-in and Quit.
//!
//! The tray lives in the Companion crate, but the window it opens belongs to the program that
//! hosts the Companion (td-app). The host hands [`start`] an [`OpenHook`] that opens its window,
//! so this crate never depends on the UI. Quit asks the Companion to stop
//! (`Handle::request_shutdown`); the host then runs its bounded shutdown and drops the [`Tray`].
use crate::api::CommonError;
use parking_lot::Mutex;
use std::sync::Arc;
#[cfg(any(windows, test))]
mod menu;
#[cfg(any(windows, test))]
mod routing;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{TRAY_ICON_RESOURCE, Tray};

/// Opens, restores and brings the host's window to the front. The tray calls it on its own
/// thread for left-click, keyboard activation and the Open item, so it must be callable from
/// any thread and must not block.
pub type OpenHook = Arc<dyn Fn() + Send + Sync>;

/// Sets the fixed, version-independent AppUserModelID for the process. Call it at startup,
/// before any window or the tray exists, so the taskbar, pins and the tray share one identity.
#[cfg(windows)]
pub fn set_app_identity() -> std::io::Result<()> {
    windows::set_app_identity()
}

/// Shows the tray icon until the returned [`Tray`] drops; dropping it removes the icon and
/// joins the tray thread within 2 s. Call it after [`set_app_identity`]. The executable must embed `app/icons/app/tray.ico` as icon resource
/// [`TRAY_ICON_RESOURCE`] and the Common Controls v6 manifest.
#[cfg(windows)]
pub fn start(handle: &crate::companion::Handle, open: OpenHook) -> Tray {
    windows::start(menu::Actions {
        handle: handle.clone(),
        open,
    })
}

/// Turns Start at sign-in on the first time an installed copy starts (`--packaged`), unless the
/// user already turned it off in Windows. Later starts leave the user's choice alone.
pub fn initialize_start_at_sign_in(
    handle: &crate::companion::Handle,
    data: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let marker = data.join("startup-initialized");
    if !marker.exists() {
        handle.desktop.startup.initialize_default()?;
        std::fs::write(marker, b"1")?;
    }
    Ok(())
}

pub trait BrowserOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), CommonError>;
}
pub trait TrayNotifier: Send + Sync {
    fn notify(&self, message: &str);
}
#[derive(Default)]
pub struct FakeBrowserOpener(pub Mutex<Option<String>>);
impl BrowserOpener for FakeBrowserOpener {
    fn open(&self, url: &str) -> Result<(), CommonError> {
        *self.0.lock() = Some(url.into());
        Ok(())
    }
}
#[derive(Default)]
pub struct FakeTrayNotifier(pub Mutex<Option<String>>);
impl TrayNotifier for FakeTrayNotifier {
    fn notify(&self, message: &str) {
        *self.0.lock() = Some(message.into());
    }
}
pub struct Desktop {
    opener: Arc<dyn BrowserOpener>,
    notifier: Arc<dyn TrayNotifier>,
    pub startup: Arc<dyn crate::startup::Startup>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub captures: Option<(Arc<FakeBrowserOpener>, Arc<FakeTrayNotifier>)>,
}
impl Desktop {
    pub(crate) fn new(mock: bool) -> Self {
        if mock {
            let opener = Arc::new(FakeBrowserOpener::default());
            let notifier = Arc::new(FakeTrayNotifier::default());
            return Self {
                opener: opener.clone(),
                notifier: notifier.clone(),
                startup: Arc::new(crate::startup::FakeStartup::default()),
                #[cfg(any(test, feature = "test-hooks"))]
                captures: Some((opener, notifier)),
            };
        }
        Self {
            opener: Arc::new(NativeBrowser),
            notifier: Arc::new(NativeNotifier),
            startup: Arc::new(crate::startup::NativeStartup),
            #[cfg(any(test, feature = "test-hooks"))]
            captures: None,
        }
    }
    pub(crate) fn deliver(&self, port: u16, code: &str) -> Result<(), CommonError> {
        self.notifier.notify(code);
        self.opener
            .open(&format!("http://127.0.0.1:{port}/pair#code={code}"))
    }
    pub(crate) fn notify(&self, message: &str) {
        self.notifier.notify(message);
    }
    #[cfg(test)]
    pub(crate) fn with_startup(mut self, startup: Arc<dyn crate::startup::Startup>) -> Self {
        self.startup = startup;
        self
    }
}
struct NativeBrowser;
impl BrowserOpener for NativeBrowser {
    fn open(&self, url: &str) -> Result<(), CommonError> {
        #[cfg(windows)]
        {
            windows::open(url)
        }
        #[cfg(not(windows))]
        {
            let _ = url;
            Err(CommonError::Internal)
        }
    }
}
struct NativeNotifier;
impl TrayNotifier for NativeNotifier {
    fn notify(&self, message: &str) {
        #[cfg(windows)]
        windows::notify(message);
        #[cfg(not(windows))]
        {
            let _ = message;
            eprintln!("Native tray notification unavailable.");
        }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
mod captures {
    use super::*;
    use crate::api::{Ctx, Endpoint, Feature, NoQuery, Registration, Req, SessionKind};
    use axum::http::Method;
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;
    #[derive(Serialize, Deserialize, TS)]
    pub struct Delivery {
        pub url: Option<String>,
        pub notification: Option<String>,
    }
    pub struct Captures;
    impl Endpoint for Captures {
        const NAME: &'static str = "test.pairing_delivery";
        const METHOD: Method = Method::GET;
        const PATH: &'static str = "/test/pairing-delivery";
        type Path = ();
        type Query = NoQuery;
        type Body = ();
        type Ok = Delivery;
        type Error = CommonError;
        async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
            if req.client().kind != SessionKind::Window {
                return Err(CommonError::Forbidden("A window session is required."));
            }
            let (opener, notifier) = ctx.desktop.captures.as_ref().ok_or(CommonError::NotFound)?;
            Ok(Delivery {
                url: opener.0.lock().take(),
                notification: notifier.0.lock().take(),
            })
        }
    }
    fn feature() -> Feature {
        Feature::new("test").endpoint::<Captures>()
    }
    inventory::submit! { Registration(feature) }
}
