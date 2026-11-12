//! Source config paths and atomic writes in Tauri's identifier folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The bundle identifier in `tauri.conf.json`, which names the app's data folders.
pub const APP_ID: &str = "io.github.duresa7.touchdeck";

pub(crate) fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The app data folder the Companion and the native window share: `TOUCHDECK_APP_DATA_DIR`
/// when set, otherwise `APP_ID` in the user's local data folder.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("TOUCHDECK_APP_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|p| p.join(APP_ID)))
}

/// Writes through a temporary file in the same directory and renames it into place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

/// Where the sources keep their configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// Holds `sources.json`.
    pub config: PathBuf,
}

impl Dirs {
    /// `TOUCHDECK_CONFIG_DIR` overrides config; otherwise a scratch app-data override
    /// holds it under `config` without reading the user's real files.
    pub fn current() -> Dirs {
        Dirs::resolve(env_dir, dirs::config_dir())
    }

    /// Resolves Tauri's config folder or the explicit config/scratch override.
    pub fn resolve(env: impl Fn(&str) -> Option<PathBuf>, config_home: Option<PathBuf>) -> Dirs {
        let config = if let Some(base) = env("TOUCHDECK_CONFIG_DIR") {
            base.join(APP_ID)
        } else if let Some(base) = env("TOUCHDECK_APP_DATA_DIR") {
            base.join("config")
        } else {
            config_home.unwrap_or_else(std::env::temp_dir).join(APP_ID)
        };
        Dirs { config }
    }

    pub fn sources_config(&self) -> PathBuf {
        self.config.join("sources.json")
    }

    pub fn app_settings(&self) -> PathBuf {
        self.config.join("app-settings.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolve(vars: &[(&str, &str)]) -> Dirs {
        let vars: HashMap<String, PathBuf> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), PathBuf::from(value)))
            .collect();
        Dirs::resolve(|name| vars.get(name).cloned(), Some("/roaming".into()))
    }

    #[test]
    fn defaults_are_the_tauri_identifier_folders() {
        let dirs = resolve(&[]);
        assert_eq!(
            dirs.sources_config(),
            Path::new("/roaming").join(APP_ID).join("sources.json")
        );
    }

    #[test]
    fn overrides_win_and_a_scratch_run_never_reaches_the_real_folders() {
        let dirs = resolve(&[
            ("TOUCHDECK_CONFIG_DIR", "/c"),
            ("TOUCHDECK_APP_DATA_DIR", "/a"),
        ]);
        assert_eq!(
            dirs.sources_config(),
            Path::new("/c").join(APP_ID).join("sources.json")
        );

        let dirs = resolve(&[("TOUCHDECK_APP_DATA_DIR", "/a")]);
        assert_eq!(dirs.sources_config(), Path::new("/a/config/sources.json"));
    }
}
