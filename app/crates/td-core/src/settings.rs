use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::Orientation;

/// Preferences for the app on this PC, separate from a Device's [`Settings`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub automatic_update_checks: bool,
    /// Whether the native editor offers Brandfetch search. Credentials stay saved when off.
    pub brandfetch_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            automatic_update_checks: true,
            brandfetch_enabled: false,
        }
    }
}

impl AppSettings {
    /// Only a missing file uses defaults. An unreadable or corrupt preference must
    /// not silently re-enable network requests. Callers should surface the error.
    pub fn load(dirs: &crate::app_dirs::Dirs) -> std::io::Result<Self> {
        match std::fs::read(dirs.app_settings()) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, dirs: &crate::app_dirs::Dirs) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::app_dirs::write_atomic(&dirs.app_settings(), &bytes)
    }

    /// Turns automatic update checks on or off, as the Settings switch does. Fields this version
    /// doesn't know stay in the file. A damaged file is an error and stays as it is, so the
    /// switch can never silently rewrite someone's preferences.
    pub fn set_automatic_update_checks(
        dirs: &crate::app_dirs::Dirs,
        on: bool,
    ) -> std::io::Result<Self> {
        Self::set_switch(dirs, "automatic_update_checks", on)
    }

    /// Changes only the Brandfetch switch, preserving other and unknown preferences.
    pub fn set_brandfetch_enabled(dirs: &crate::app_dirs::Dirs, on: bool) -> std::io::Result<Self> {
        Self::set_switch(dirs, "brandfetch_enabled", on)
    }

    fn set_switch(dirs: &crate::app_dirs::Dirs, field: &str, on: bool) -> std::io::Result<Self> {
        // All switch owners share the read-modify-write gate.
        static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _write = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let path = dirs.app_settings();
        let mut fields = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(fields)) => {
                    // The file must read as settings before anything in it changes.
                    serde_json::from_value::<Self>(Value::Object(fields.clone()))
                        .map_err(std::io::Error::other)?;
                    fields
                }
                Ok(_) => {
                    return Err(std::io::Error::other(
                        "app-settings.json doesn't hold an object",
                    ));
                }
                Err(error) => return Err(std::io::Error::other(error)),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(error) => return Err(error),
        };
        fields.insert(field.into(), Value::Bool(on));
        let value = Value::Object(fields);
        let settings: Self =
            serde_json::from_value(value.clone()).map_err(std::io::Error::other)?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let bytes = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
        crate::app_dirs::write_atomic(&path, &bytes)?;
        Ok(settings)
    }
}

pub const IDLE_TIMEOUT_MIN_S: u32 = 1;
pub const IDLE_TIMEOUT_MAX_S: u32 = 86400;

/// The deck settings stored in `/storage/settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub idle_timeout_s: u32,
    pub orientation: Orientation,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            idle_timeout_s: 600,
            orientation: Orientation::Landscape,
        }
    }
}

impl Settings {
    /// Reads settings the way the firmware does: an unknown field is ignored, and a
    /// missing or out-of-range field takes its default.
    pub fn from_value(value: &Value) -> Settings {
        let defaults = Settings::default();
        let idle_timeout_s = value
            .get("idle_timeout_s")
            .and_then(Value::as_u64)
            .and_then(|timeout| u32::try_from(timeout).ok())
            .filter(|timeout| (IDLE_TIMEOUT_MIN_S..=IDLE_TIMEOUT_MAX_S).contains(timeout))
            .unwrap_or(defaults.idle_timeout_s);
        let orientation = match value.get("orientation").and_then(Value::as_str) {
            Some("portrait") => Orientation::Portrait,
            _ => Orientation::Landscape,
        };
        Settings {
            idle_timeout_s,
            orientation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dirs(folder: &std::path::Path) -> crate::app_dirs::Dirs {
        crate::app_dirs::Dirs {
            config: folder.join("config"),
        }
    }

    #[test]
    fn the_brandfetch_switch_defaults_off_and_preserves_other_preferences() {
        let folder = tempfile::tempdir().unwrap();
        let dirs = dirs(folder.path());
        assert!(!AppSettings::load(&dirs).unwrap().brandfetch_enabled);
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(
            dirs.app_settings(),
            br#"{"automatic_update_checks":false,"future":[1,2]}"#,
        )
        .unwrap();
        assert!(
            AppSettings::set_brandfetch_enabled(&dirs, true)
                .unwrap()
                .brandfetch_enabled
        );
        AppSettings::set_automatic_update_checks(&dirs, true).unwrap();
        assert!(AppSettings::load(&dirs).unwrap().brandfetch_enabled);
        assert!(
            !AppSettings::set_brandfetch_enabled(&dirs, false)
                .unwrap()
                .brandfetch_enabled
        );
        let written: Value =
            serde_json::from_slice(&std::fs::read(dirs.app_settings()).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({"automatic_update_checks":true,"brandfetch_enabled":false,"future":[1,2]})
        );
        for damaged in ["{bad", "[]", r#"{"brandfetch_enabled":"yes"}"#] {
            std::fs::write(dirs.app_settings(), damaged).unwrap();
            assert!(AppSettings::set_brandfetch_enabled(&dirs, true).is_err());
            assert_eq!(
                std::fs::read_to_string(dirs.app_settings()).unwrap(),
                damaged
            );
        }
    }

    #[test]
    fn the_update_switch_writes_its_field_and_keeps_the_others() {
        let folder = tempfile::tempdir().unwrap();
        let dirs = dirs(folder.path());
        assert!(AppSettings::load(&dirs).unwrap().automatic_update_checks);

        let off = AppSettings::set_automatic_update_checks(&dirs, false).unwrap();
        assert!(!off.automatic_update_checks);
        assert!(!AppSettings::load(&dirs).unwrap().automatic_update_checks);

        std::fs::write(
            dirs.app_settings(),
            br#"{"automatic_update_checks": false, "later_field": [1, 2]}"#,
        )
        .unwrap();
        AppSettings::set_automatic_update_checks(&dirs, true).unwrap();
        let written: Value =
            serde_json::from_slice(&std::fs::read(dirs.app_settings()).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({"automatic_update_checks": true, "later_field": [1, 2]})
        );
    }

    #[test]
    fn the_update_switch_leaves_a_damaged_file_alone() {
        let folder = tempfile::tempdir().unwrap();
        let dirs = dirs(folder.path());
        std::fs::create_dir_all(&dirs.config).unwrap();
        for damaged in [
            &b"{not json"[..],
            b"[true]",
            br#"{"automatic_update_checks": "yes"}"#,
        ] {
            std::fs::write(dirs.app_settings(), damaged).unwrap();
            assert!(AppSettings::set_automatic_update_checks(&dirs, true).is_err());
            assert_eq!(std::fs::read(dirs.app_settings()).unwrap(), damaged);
        }
    }

    #[test]
    fn firmware_parsing_rules() {
        assert_eq!(Settings::from_value(&json!({})), Settings::default());
        assert_eq!(
            Settings::from_value(
                &json!({"idle_timeout_s": 120, "orientation": "portrait", "extra": 1})
            ),
            Settings {
                idle_timeout_s: 120,
                orientation: Orientation::Portrait
            }
        );
        assert_eq!(
            Settings::from_value(&json!({"idle_timeout_s": 0})).idle_timeout_s,
            600
        );
        assert_eq!(
            Settings::from_value(&json!({"idle_timeout_s": 86401})).idle_timeout_s,
            600
        );
        assert_eq!(
            Settings::from_value(&json!({"orientation": "sideways"})).orientation,
            Orientation::Landscape
        );
    }
}
