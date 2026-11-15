//! What the window remembers between runs: where it was and whether the close notice has shown.
//! It lives in `window.json` in the app data folder, beside the Companion's own files.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file name inside the app data folder.
pub const FILE: &str = "window.json";
/// The window's minimum size in effective pixels. `ui/app.slint` sets the same values.
pub const MIN_WIDTH: f32 = 500.0;
pub const MIN_HEIGHT: f32 = 360.0;
/// Larger saved sizes are treated as damaged.
const MAX_SIDE: f32 = 32_768.0;

/// The window's last normal (not maximised or minimised) place, and whether it was maximised.
/// The position is in physical pixels, as the platform reports it; the size is in effective
/// pixels, so it survives a change of display scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}
impl Placement {
    /// Keeps a usable size: at least the window's minimum, and finite.
    fn sanitized(self) -> Option<Self> {
        let valid = |side: f32| side.is_finite() && side > 0.0 && side <= MAX_SIDE;
        (valid(self.width) && valid(self.height)).then(|| Self {
            width: self.width.max(MIN_WIDTH),
            height: self.height.max(MIN_HEIGHT),
            ..self
        })
    }
}

/// Light or dark, as Settings > Appearance chooses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
    /// Follow the system's light or dark setting. Also what an unknown value reads as.
    #[default]
    #[serde(other)]
    System,
}
impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPrefs {
    pub placement: Option<Placement>,
    /// The one-time notice that closing keeps the app in the tray has been shown.
    pub close_notice_shown: bool,
    pub appearance: Appearance,
    /// The person folded the editor's Page filmstrip in a wide window.
    pub filmstrip_folded: bool,
    /// The Device the editor showed last, so it opens there next time.
    pub editor_device: Option<String>,
}

/// The preferences and the file they came from.
#[derive(Clone, Debug)]
pub struct Prefs {
    path: PathBuf,
    pub value: WindowPrefs,
}
impl Prefs {
    /// Reads `window.json` from `dir`. A missing or unreadable file gives the defaults: the
    /// window then opens at its preferred size and shows the notice again, which is harmless.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let value = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<WindowPrefs>(&bytes).unwrap_or_else(|error| {
                eprintln!("Open TouchDeck: ignoring {}: {error}", path.display());
                WindowPrefs::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => WindowPrefs::default(),
            Err(error) => {
                eprintln!("Open TouchDeck: could not read {}: {error}", path.display());
                WindowPrefs::default()
            }
        };
        let value = WindowPrefs {
            placement: value.placement.and_then(Placement::sanitized),
            ..value
        };
        Self { path, value }
    }

    /// Writes through a temporary file, so a crash mid-write never leaves half a file.
    pub fn save(&self) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.value).map_err(std::io::Error::other)?;
        td_core::app_dirs::write_atomic(&self.path, &bytes)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement() -> Placement {
        Placement {
            x: -1200,
            y: 80,
            width: 1024.0,
            height: 700.5,
            maximized: true,
        }
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = Prefs::load(dir.path());
        assert_eq!(prefs.value, WindowPrefs::default());
        assert_eq!(prefs.path(), dir.path().join(FILE));
        assert!(!dir.path().join(FILE).exists(), "loading never writes");
    }

    #[test]
    fn placement_and_the_notice_flag_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut prefs = Prefs::load(dir.path());
        prefs.value.placement = Some(placement());
        prefs.value.close_notice_shown = true;
        prefs.save().unwrap();

        let again = Prefs::load(dir.path());
        assert_eq!(again.value.placement, Some(placement()));
        assert!(again.value.close_notice_shown);
    }

    #[test]
    fn saving_creates_the_data_folder_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("io.github.duresa7.touchdeck");
        let mut prefs = Prefs::load(&data);
        prefs.value.close_notice_shown = true;
        prefs.save().unwrap();
        let names: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [FILE]);
    }

    #[test]
    fn a_damaged_file_gives_the_defaults_and_the_next_save_repairs_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), b"{\"placement\": {\"x\": ").unwrap();
        let mut prefs = Prefs::load(dir.path());
        assert_eq!(prefs.value, WindowPrefs::default());

        prefs.value.close_notice_shown = true;
        prefs.save().unwrap();
        assert!(Prefs::load(dir.path()).value.close_notice_shown);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE),
            br#"{"close_notice_shown": true, "added_later": [1, 2]}"#,
        )
        .unwrap();
        let prefs = Prefs::load(dir.path());
        assert!(prefs.value.close_notice_shown);
        assert_eq!(prefs.value.placement, None);
    }

    #[test]
    fn the_editor_choices_survive_a_restart_and_older_files_lack_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), br#"{"close_notice_shown": true}"#).unwrap();
        let mut prefs = Prefs::load(dir.path());
        assert!(!prefs.value.filmstrip_folded);
        assert_eq!(prefs.value.editor_device, None);

        prefs.value.filmstrip_folded = true;
        prefs.value.editor_device = Some("mock0002".into());
        prefs.save().unwrap();
        let again = Prefs::load(dir.path()).value;
        assert!(again.filmstrip_folded);
        assert_eq!(again.editor_device.as_deref(), Some("mock0002"));
        assert!(again.close_notice_shown);
    }

    #[test]
    fn a_saved_size_below_the_minimum_grows_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut prefs = Prefs::load(dir.path());
        prefs.value.placement = Some(Placement {
            width: 120.0,
            height: 90.0,
            ..placement()
        });
        prefs.save().unwrap();
        let placement = Prefs::load(dir.path()).value.placement.unwrap();
        assert_eq!((placement.width, placement.height), (MIN_WIDTH, MIN_HEIGHT));
        assert_eq!(
            (placement.x, placement.y),
            (-1200, 80),
            "the position stays"
        );
    }

    #[test]
    fn a_nonsensical_size_drops_the_placement() {
        for (width, height) in [(0.0, 700.0), (800.0, -1.0), (1e9, 700.0)] {
            let dir = tempfile::tempdir().unwrap();
            let mut prefs = Prefs::load(dir.path());
            prefs.value.placement = Some(Placement {
                width,
                height,
                ..placement()
            });
            prefs.value.close_notice_shown = true;
            prefs.save().unwrap();
            let again = Prefs::load(dir.path());
            assert_eq!(again.value.placement, None, "{width}x{height}");
            assert!(again.value.close_notice_shown, "the flag is kept");
        }
    }
}
