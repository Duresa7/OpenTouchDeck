//! The desktop portal's text size and contrast settings, read and followed through
//! `org.freedesktop.portal.Settings`. Slint reads the colour scheme and accent the same way.
use super::Look;
use crate::AppWindow;
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedValue, Value},
};

const APPEARANCE: &str = "org.freedesktop.appearance";
const INTERFACE: &str = "org.gnome.desktop.interface";
const TEXT_SCALE: &str = "text-scaling-factor";
const CONTRAST: &str = "contrast";

/// Follows the portal on its own thread. Without a session bus or portal, the defaults stay.
pub(super) fn watch(ui: slint::Weak<AppWindow>) {
    let spawned = std::thread::Builder::new()
        .name("desktop-settings".into())
        .spawn(move || {
            if let Err(error) = follow(&ui) {
                eprintln!("Open TouchDeck: desktop settings unavailable: {error}");
            }
        });
    if let Err(error) = spawned {
        eprintln!("Open TouchDeck: desktop settings unavailable: {error}");
    }
}

fn follow(ui: &slint::Weak<AppWindow>) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let settings = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )?;
    // Subscribe before reading, so a change in between isn't lost.
    let changes = settings.receive_signal("SettingChanged")?;
    let mut look = Look::default();
    for (namespace, key) in [(INTERFACE, TEXT_SCALE), (APPEARANCE, CONTRAST)] {
        if let Some(value) = read(&settings, namespace, key) {
            set(&mut look, namespace, key, &value);
        }
    }
    if !push(ui, &look) {
        return Ok(());
    }
    for message in changes {
        let Ok((namespace, key, value)) =
            message.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        if set(&mut look, &namespace, &key, &value) && !push(ui, &look) {
            break;
        }
    }
    Ok(())
}

/// `ReadOne` where the portal has it; older portals only have `Read`, which wraps the value
/// in one more variant. `downcast_ref` looks through that wrapping.
fn read(settings: &Proxy, namespace: &str, key: &str) -> Option<OwnedValue> {
    settings
        .call::<_, _, OwnedValue>("ReadOne", &(namespace, key))
        .or_else(|_| settings.call::<_, _, OwnedValue>("Read", &(namespace, key)))
        .ok()
}

/// Shows the look; false once the window has gone.
fn push(ui: &slint::Weak<AppWindow>, look: &Look) -> bool {
    let look = look.clone();
    ui.upgrade_in_event_loop(move |ui| super::apply(&ui, &look))
        .is_ok()
}

/// Applies one portal setting. True when it changed the look.
pub(super) fn set(look: &mut Look, namespace: &str, key: &str, value: &Value<'_>) -> bool {
    let before = look.clone();
    match (namespace, key) {
        (INTERFACE, TEXT_SCALE) => {
            if let Ok(scale) = value.downcast_ref::<f64>() {
                look.text_scale = scale as f32;
            }
        }
        // 0: no preference, 1: higher contrast.
        (APPEARANCE, CONTRAST) => {
            if let Ok(contrast) = value.downcast_ref::<u32>() {
                look.high_contrast = contrast == 1;
            }
        }
        _ => {}
    }
    *look != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_and_contrast_follow_the_portal() {
        let mut look = Look::default();
        assert!(set(&mut look, INTERFACE, TEXT_SCALE, &Value::from(1.5f64)));
        assert_eq!(look.text_scale, 1.5);
        assert!(set(&mut look, APPEARANCE, CONTRAST, &Value::from(1u32)));
        assert!(look.high_contrast);
        assert!(set(&mut look, APPEARANCE, CONTRAST, &Value::from(0u32)));
        assert!(!look.high_contrast);
    }

    #[test]
    fn a_value_wrapped_by_the_old_read_method_is_understood() {
        let mut look = Look::default();
        let wrapped = Value::Value(Box::new(Value::from(2.0f64)));
        assert!(set(&mut look, INTERFACE, TEXT_SCALE, &wrapped));
        assert_eq!(look.text_scale, 2.0);
    }

    #[test]
    fn other_settings_and_wrong_types_change_nothing() {
        let mut look = Look::default();
        assert!(!set(
            &mut look,
            INTERFACE,
            "font-name",
            &Value::from("Sans 11")
        ));
        assert!(!set(&mut look, INTERFACE, TEXT_SCALE, &Value::from("big")));
        assert!(!set(&mut look, APPEARANCE, CONTRAST, &Value::from(1.0f64)));
        assert!(!set(&mut look, INTERFACE, TEXT_SCALE, &Value::from(1.0f64)));
        assert_eq!(look, Look::default());
    }
}
