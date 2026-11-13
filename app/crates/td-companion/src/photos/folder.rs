use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

const EXTENSIONS: [&str; 3] = ["jpg", "jpeg", "png"];
pub(crate) const FILE_BYTES_MAX: u64 = 64 * 1024 * 1024;

fn is_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub(crate) fn pictures(folder: &Path) -> Vec<PathBuf> {
    let Ok(root) = folder.canonicalize() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.path())
        .filter(|path| is_picture(path))
        .collect();
    found.sort();
    found
}

pub(crate) fn read(folder: &Path, path: &Path) -> io::Result<Vec<u8>> {
    let outside = || io::Error::other("not a picture file in the chosen folder");
    let root = folder.canonicalize()?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > FILE_BYTES_MAX || !is_picture(path) {
        return Err(outside());
    }
    let real = path.canonicalize()?;
    if real.parent() != Some(root.as_path()) {
        return Err(outside());
    }
    let mut data = Vec::new();
    fs::File::open(&real)?
        .take(FILE_BYTES_MAX + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > FILE_BYTES_MAX {
        return Err(outside());
    }
    Ok(data)
}

pub(crate) fn valid(folder: &str) -> bool {
    let path = Path::new(folder);
    path.is_absolute() && path.is_dir()
}

/// Asks the person for a picture folder with the system's folder dialog. Blocking: run it off
/// the UI thread. `Ok(None)` when they cancel.
pub fn pick() -> Result<Option<String>, String> {
    #[cfg(windows)]
    let output = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-STA",
                "-Command",
                r#"Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Choose a folder of pictures'; $d.UseDescriptionForTitle = $true; if ($d.ShowDialog() -eq 'OK') { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; [Console]::Write($d.SelectedPath) }; $d.Dispose()"#,
            ])
            // CREATE_NO_WINDOW: no console flashes up behind the dialog.
            .creation_flags(0x08000000)
            .output()
    };
    #[cfg(not(windows))]
    let output = std::process::Command::new("zenity")
        .args([
            "--file-selection",
            "--directory",
            "--title=Choose a folder of pictures",
        ])
        .output();
    let output = output.map_err(|_| "Could not open the folder dialog.".to_string())?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((output.status.success() && !path.is_empty()).then_some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(target: &Path, at: &Path) -> bool {
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(target, at);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, at);
        made.is_ok()
    }

    #[test]
    fn only_regular_picture_files_directly_in_the_folder_are_listed_and_read() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.jpg");
        fs::write(&secret, b"outside the folder").unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let dir = chosen.path();
        for name in ["b.JPG", "a.png", "c.jpeg", "notes.txt", "noext"] {
            fs::write(dir.join(name), name).unwrap();
        }
        fs::create_dir(dir.join("nested.jpg")).unwrap();
        fs::write(dir.join("nested.jpg").join("inner.jpg"), b"nested").unwrap();
        let linked = link(&secret, &dir.join("escape.jpg"));

        let names: Vec<String> = pictures(dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.png", "b.JPG", "c.jpeg"]);
        for path in pictures(dir) {
            assert!(read(dir, &path).is_ok());
        }

        assert!(read(dir, &dir.join("notes.txt")).is_err());
        assert!(read(dir, &dir.join("nested.jpg").join("inner.jpg")).is_err());
        assert!(read(dir, &dir.join("..").join("x.jpg")).is_err());
        assert!(read(dir, &secret).is_err());
        if linked {
            assert!(read(dir, &dir.join("escape.jpg")).is_err());
        } else {
            eprintln!("symlinks unavailable here; the link case ran only as a missing file");
        }
    }

    #[test]
    fn a_missing_or_relative_folder_has_no_pictures_and_cannot_be_chosen() {
        let gone = tempfile::tempdir().unwrap().path().to_owned();
        assert!(pictures(&gone).is_empty());
        assert!(!valid(&gone.to_string_lossy()));
        assert!(!valid("photos"));
        let here = tempfile::tempdir().unwrap();
        assert!(valid(&here.path().to_string_lossy()));
    }
}
