# Windows evidence

These scripts prove Open TouchDeck 2.0's M1 (issue #353) on a real Windows desktop. The
`windows evidence` workflow runs them on a GitHub-hosted runner after a merge to `main` that
changes the app window or the tray, or by hand. They are not a pull request check.

| Script | What it does |
| --- | --- |
| `run.ps1` | Runs the four parts below in order, then `summary.ps1`. |
| `environment.ps1` | Records the OS build, the desktop session, the display adapter, the fonts and the theme settings. |
| `look.ps1` | Screenshots of Home and About in light, dark, a contrast theme, 200% text and at the minimum width. Dumps the UI Automation tree of both pages. |
| `memory.ps1` | Working set and private bytes with one mock Device. |
| `tray.ps1` | The eight notification-area checks. Uses `aumid.ps1` to ask the shell for the window's AppUserModelID. |
| `summary.ps1` | Writes `summary.md` and `results.json`, fills the job summary, and fails when a check failed. |
| `lib.ps1` | Shared Win32 calls, screenshots, UI Automation and the check recorder. |

## Run them by hand

They change the signed-in user's theme, contrast theme, text size and Start at sign-in values,
restart Explorer, and put nothing back. Use a machine you can throw away, from a desktop
session, in Windows PowerShell 5.1:

```powershell
cd app
cargo build --release --locked --package td-app --features mock
cd ..
powershell -ExecutionPolicy Bypass -File .github/scripts/windows-evidence/run.ps1 -Exe app/target/release/td-app.exe -Out evidence
```

Each part also runs alone with the same `-Exe` and `-Out`.

## How the scripts act as a user

- **Settings.** Light and dark, and the text size, are the registry values Settings writes
  followed by the `WM_SETTINGCHANGE` broadcast. The contrast theme is
  `SystemParametersInfo(SPI_SETHIGHCONTRAST)`, which is what the Left Alt + Left Shift +
  Print Screen shortcut calls.
- **The window.** Pages are opened and buttons pressed with real mouse clicks at the place UI
  Automation reports. Screenshots are copied from the screen, so they show what DWM composes.
- **The tray.** A notification icon gets its events as a window message, so the scripts post
  the message the Shell would send (`WM_APP+1` with `NIN_SELECT`, `NIN_KEYSELECT` or
  `WM_CONTEXTMENU`) to the hidden `OpenTouchDeckTray` window. The popup menu that follows is the
  real one: its items are read through UI Automation and chosen with a mouse click.

## What a GitHub-hosted runner can and cannot prove

`windows-latest` is Windows Server 2025, not Windows 11. `environment.json` and the job summary
record what each run found. See the comment at the top of
[`windows-evidence.yml`](../../workflows/windows-evidence.yml).
