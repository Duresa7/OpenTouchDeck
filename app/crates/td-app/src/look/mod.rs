//! The platform's look beyond what Slint follows itself. Slint's Fluent style already tracks
//! light and dark and the accent colour (the DWM colour on Windows, the portal on Linux). This
//! adds the text size setting, contrast themes, Segoe UI Variable and, on Windows 11, Mica.
use crate::{AppWindow, ContrastColors, Theme};
use slint::ComponentHandle;

#[cfg(all(unix, not(target_vendor = "apple")))]
mod portal;
#[cfg(windows)]
mod win;

/// What the platform asks for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Look {
    /// The text size setting; 1.0 is 100%.
    pub text_scale: f32,
    /// A contrast theme is on.
    pub high_contrast: bool,
    /// The system's contrast colours, where it has them (Windows).
    pub contrast: Option<ContrastColors>,
}
impl Default for Look {
    fn default() -> Self {
        Self {
            text_scale: 1.0,
            high_contrast: false,
            contrast: None,
        }
    }
}

/// Windows offers 100% to 225% and GNOME 50% to 300%. Anything outside is a mistake.
fn text_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(0.5, 3.0)
    } else {
        1.0
    }
}

pub(crate) fn apply(ui: &AppWindow, look: &Look) {
    let theme = ui.global::<Theme>();
    theme.set_text_scale(text_scale(look.text_scale));
    theme.set_high_contrast(look.high_contrast);
    theme.set_has_system_contrast(look.contrast.is_some());
    if let Some(colors) = &look.contrast {
        theme.set_system_contrast(colors.clone());
    }
}

/// Segoe UI Variable on Windows: its Text cut for body text, its Display cut for headings.
/// Elsewhere the desktop's UI font.
pub(crate) fn fonts(ui: &AppWindow) {
    #[cfg(windows)]
    {
        let theme = ui.global::<Theme>();
        theme.set_text_font("Segoe UI Variable Text".into());
        theme.set_display_font("Segoe UI Variable Display".into());
    }
    #[cfg(not(windows))]
    let _ = ui;
}

/// Selects Slint's platform. On Windows the window is created with the app icon from the
/// executable and, on Windows 11 22H2 and later, transparent so Mica can show behind it;
/// elsewhere Slint's default (or `SLINT_BACKEND`) applies.
pub fn init_platform() -> Result<(), slint::PlatformError> {
    #[cfg(windows)]
    {
        let mica = win::mica_supported();
        slint::BackendSelector::new()
            .with_winit_window_attributes_hook(move |attributes| {
                let attributes = win::with_app_icon(attributes);
                if mica {
                    attributes.with_transparent(true)
                } else {
                    attributes
                }
            })
            .select()
    }
    #[cfg(not(windows))]
    Ok(())
}

/// Follows the platform's settings until the returned value drops.
pub(crate) struct Watch {
    #[cfg(windows)]
    _settings: Option<win::Watch>,
}

pub(crate) fn watch(ui: &AppWindow) -> Watch {
    #[cfg(windows)]
    {
        apply(ui, &win::read());
        Watch {
            _settings: win::watch(ui.as_weak()),
        }
    }
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        portal::watch(ui.as_weak());
        Watch {}
    }
    #[cfg(not(any(windows, all(unix, not(target_vendor = "apple")))))]
    {
        let _ = ui;
        Watch {}
    }
}

/// Once the window exists: the app icon and, on Windows 11 22H2 and later, Mica; a solid
/// background elsewhere.
pub(crate) fn shown(ui: &AppWindow) {
    #[cfg(windows)]
    {
        win::app_icon(ui.window());
        let theme = ui.global::<Theme>();
        theme.set_backdrop(win::backdrop(ui.window(), theme.get_dark()));
    }
    #[cfg(not(windows))]
    let _ = ui;
}

/// Light and dark switched: Windows redraws Mica and the title bar to match.
pub(crate) fn dark_changed(ui: &AppWindow, dark: bool) {
    #[cfg(windows)]
    win::dark(ui.window(), dark);
    #[cfg(not(windows))]
    let _ = (ui, dark);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_stays_in_the_range_platforms_offer() {
        assert_eq!(text_scale(1.0), 1.0);
        assert_eq!(text_scale(2.25), 2.25);
        assert_eq!(text_scale(0.1), 0.5);
        assert_eq!(text_scale(9.0), 3.0);
        assert_eq!(text_scale(f32::NAN), 1.0);
        assert_eq!(text_scale(f32::INFINITY), 1.0);
    }
}
