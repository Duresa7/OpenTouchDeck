//! Windows 11: Mica through DWM, the text size setting through UISettings, and contrast themes
//! through `SPI_GETHIGHCONTRAST` and the system colours. See docs/research/windows-11-native.md.
use super::Look;
use crate::{AppWindow, ContrastColors};
use slint::winit_030::{
    WinitWindowAccessor,
    winit::{
        dpi::PhysicalSize,
        platform::windows::{IconExtWindows, WindowAttributesExtWindows, WindowExtWindows},
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
        window::{Icon, WindowAttributes},
    },
};
use windows::{
    Foundation::TypedEventHandler,
    UI::ViewManagement::{AccessibilitySettings, UISettings},
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::HWND,
        Graphics::{
            Dwm::{
                DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
                DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
            },
            Gdi::{
                COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
                COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
            },
        },
        System::SystemInformation::OSVERSIONINFOW,
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            Controls::MARGINS,
            WindowsAndMessaging::{
                GetSystemMetrics, SM_CXICON, SM_CXSMICON, SPI_GETHIGHCONTRAST,
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
            },
        },
    },
    core::BOOL,
};

/// `DWMWA_SYSTEMBACKDROP_TYPE` is documented from Windows 11 build 22621 (22H2).
const MICA_BUILD: u32 = 22621;

fn build() -> u32 {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // RtlGetVersion reports the real build, whatever the manifest claims compatibility with.
    if unsafe { RtlGetVersion(&mut info) }.is_ok() {
        info.dwBuildNumber
    } else {
        0
    }
}

pub(super) fn mica_supported() -> bool {
    build() >= MICA_BUILD
}

fn hwnd(window: &slint::Window) -> Option<HWND> {
    window
        .with_winit_window(|window| match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
            _ => None,
        })
        .flatten()
}

fn set_attribute<T>(
    hwnd: HWND,
    attribute: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE,
    value: &T,
) -> bool {
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute,
            (value as *const T).cast(),
            size_of::<T>() as u32,
        )
    }
    .is_ok()
}

/// The app icon's resource ID in td-app's executable (`build.rs`).
const APP_ICON_RESOURCE: u16 = 1;

/// The app icon from the executable, at the system's small (title bar) and large (Alt+Tab,
/// taskbar) sizes, each picked from `open-touchdeck.ico` rather than scaled from one size.
fn app_icons() -> (Option<Icon>, Option<Icon>) {
    let load = |metric| {
        let side = u32::try_from(unsafe { GetSystemMetrics(metric) }).unwrap_or(0);
        let size = (side > 0).then(|| PhysicalSize::new(side, side));
        Icon::from_resource(APP_ICON_RESOURCE, size).ok()
    };
    (load(SM_CXSMICON), load(SM_CXICON))
}

/// Gives a new window the app icon, so it has it from its first frame.
pub(super) fn with_app_icon(attributes: WindowAttributes) -> WindowAttributes {
    let (small, large) = app_icons();
    attributes.with_window_icon(small).with_taskbar_icon(large)
}

/// Sets the app icon again once the window exists, in case Slint reset it.
pub(super) fn app_icon(window: &slint::Window) {
    window.with_winit_window(|window| {
        let (small, large) = app_icons();
        window.set_window_icon(small);
        window.set_taskbar_icon(large);
    });
}

/// Asks DWM for Mica behind the whole window. False means it isn't available, so the window
/// keeps a solid background.
pub(super) fn backdrop(window: &slint::Window, dark: bool) -> bool {
    if !mica_supported() {
        #[cfg(feature = "mock")]
        eprintln!(
            "Open TouchDeck (mock): Mica: not asked for on build {}",
            build()
        );
        return false;
    }
    let Some(hwnd) = hwnd(window) else {
        #[cfg(feature = "mock")]
        eprintln!("Open TouchDeck (mock): Mica: the window has no handle");
        return false;
    };
    set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &BOOL::from(dark));
    // Extending the frame over the client area lets the backdrop show through it.
    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    let extended = unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) };
    let backdrop = extended.is_ok()
        && set_attribute::<DWM_SYSTEMBACKDROP_TYPE>(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &DWMSBT_MAINWINDOW,
        );
    // The Windows evidence scripts read this line (.github/scripts/windows-evidence).
    #[cfg(feature = "mock")]
    eprintln!("Open TouchDeck (mock): Mica: frame extended {extended:?}, backdrop set {backdrop}");
    backdrop
}

/// Light or dark Mica and title bar.
pub(super) fn dark(window: &slint::Window, dark: bool) {
    if let Some(hwnd) = hwnd(window) {
        set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &BOOL::from(dark));
    }
}

fn high_contrast() -> bool {
    let mut contrast = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
}

fn system_color(index: SYS_COLOR_INDEX) -> slint::Color {
    // A COLORREF is 0x00BBGGRR.
    let [r, g, b, _] = unsafe { GetSysColor(index) }.to_le_bytes();
    slint::Color::from_rgb_u8(r, g, b)
}

/// The colours of the contrast theme, including any the user customised.
fn contrast_colors() -> ContrastColors {
    ContrastColors {
        window: system_color(COLOR_WINDOW),
        window_text: system_color(COLOR_WINDOWTEXT),
        highlight: system_color(COLOR_HIGHLIGHT),
        highlight_text: system_color(COLOR_HIGHLIGHTTEXT),
        hotlight: system_color(COLOR_HOTLIGHT),
        gray_text: system_color(COLOR_GRAYTEXT),
        button_face: system_color(COLOR_BTNFACE),
        button_text: system_color(COLOR_BTNTEXT),
    }
}

/// The text size setting and contrast theme as they are now.
pub(super) fn read() -> Look {
    let text_scale = UISettings::new()
        .and_then(|settings| settings.TextScaleFactor())
        .unwrap_or(1.0) as f32;
    let high_contrast = high_contrast();
    Look {
        text_scale,
        high_contrast,
        contrast: high_contrast.then(contrast_colors),
    }
}

/// Keeps the change subscriptions; dropping it unsubscribes.
pub(super) struct Watch {
    settings: UISettings,
    accessibility: AccessibilitySettings,
    text_scale: i64,
    colors: i64,
    contrast: i64,
}
impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.settings.RemoveTextScaleFactorChanged(self.text_scale);
        let _ = self.settings.RemoveColorValuesChanged(self.colors);
        let _ = self.accessibility.RemoveHighContrastChanged(self.contrast);
    }
}

/// Rereads everything when the text size, the system colours or the contrast theme change.
/// The events arrive on a background thread, so the look reaches the window through its event
/// loop.
pub(super) fn watch(ui: slint::Weak<AppWindow>) -> Result<Watch, String> {
    let changed = move || {
        let look = read();
        #[cfg(feature = "mock")]
        eprintln!(
            "Open TouchDeck (mock): settings changed: text scale {}, contrast theme {}",
            look.text_scale, look.high_contrast
        );
        let _ = ui.upgrade_in_event_loop(move |ui| super::apply(&ui, &look));
    };
    // The step that failed goes into the error, so a log line names it.
    let mut step = "UISettings";
    let watch = (|| -> windows::core::Result<Watch> {
        let settings = UISettings::new()?;
        step = "AccessibilitySettings";
        let accessibility = AccessibilitySettings::new()?;
        step = "TextScaleFactorChanged";
        let on_text = changed.clone();
        let text_scale =
            settings.TextScaleFactorChanged(&TypedEventHandler::new(move |_, _| {
                on_text();
                Ok(())
            }))?;
        step = "ColorValuesChanged";
        let on_colors = changed.clone();
        let colors = settings.ColorValuesChanged(&TypedEventHandler::new(move |_, _| {
            on_colors();
            Ok(())
        }))?;
        step = "HighContrastChanged";
        let contrast =
            accessibility.HighContrastChanged(&TypedEventHandler::new(move |_, _| {
                changed();
                Ok(())
            }))?;
        Ok(Watch {
            settings,
            accessibility,
            text_scale,
            colors,
            contrast,
        })
    })();
    watch.map_err(|error| format!("{step}: {error}"))
}
