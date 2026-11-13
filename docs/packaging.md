# Windows installer and app updates

End users download the [Windows x64 installer](https://github.com/Duresa7/OpenTouchDeck/releases/latest/download/OpenTouchDeck-windows-x64-setup.exe) from the latest GitHub release and run it. It installs for the current user only and needs no administrator rights, Node, Rust, Python or ESP-IDF.

## What it installs

The installer is built with [electron-builder](https://www.electron.build/nsis) (NSIS, assisted mode, per user). It always installs to `%LOCALAPPDATA%\Programs\Open TouchDeck`; the folder can't be changed, so the app never lands somewhere other users can write.

| Path | What it is |
|---|---|
| `Open TouchDeck.exe` | The Electron window |
| `td-companion.exe` | The Companion, with the UI and the normal Firmware bundle built in. When the release ships the opt-in radio build (see [The radio build in a release](#the-radio-build-in-a-release)), it is built in too, so switching works offline |
| `resources\app.asar` | The window's code |
| `resources\firmware\touchdeck.tdfw` | The same normal-build bundle the Companion carries, for picking by hand. The radio build has no copy here. |
| `resources\install-manifest.json` | Source revision, version, and each Firmware bundle's hash; a radio build is recorded as embedded in `td-companion.exe` |
| `resources\bridge.ps1` | The bridge to 2.0 (see [The bridge to 2.0](#the-bridge-to-20)) |
| `resources\LICENSE.txt`, `resources\licenses\` | Licenses |
| `Uninstall Open TouchDeck.exe` | The uninstaller |

It adds Start menu and desktop shortcuts and an entry in **Installed apps**. On its first start the Companion adds the per-user Run value `Open TouchDeck` so it starts at sign-in (see [companion-api.md](companion-api.md#tray-and-startup-implementation-decisions-79)).

User data stays outside the install folder: `%LOCALAPPDATA%\io.github.duresa7.touchdeck` (Companion data and its runtime file), `%APPDATA%\io.github.duresa7.touchdeck` and `%APPDATA%\Open TouchDeck` (the window).

## Install, update and uninstall

- **Install or reinstall.** Before it replaces files, the installer stops a running Companion through its own shutdown route: it reads the runtime file, checks the Companion's HMAC proof before it sends the pairing token, takes `system.lock` and calls `system.shutdown`. It waits up to 15 seconds, then ends only processes whose path is exactly the installed `td-companion.exe`. If a Device is busy with a Job such as a Firmware update, the installer stops and says so. It then closes the window.
- **Downgrade.** An installer older than the installed version refuses to run (exit code 5). Uninstall first to go back.
- **Update.** Open TouchDeck checks `latest.json`, downloads the installer, and checks its signature with the updater key built into the app (ADR 0005) before and right before running it. It takes `system.lock`, stops the Companion, starts the installer with `/S --updated --force-run` and quits. The installer replaces the files silently and starts Open TouchDeck again.
- **Update to 2.0.** On Windows 11, once 1.x is up to date, the update offered is Open TouchDeck 2.0. See [The bridge to 2.0](#the-bridge-to-20).
- **Uninstall.** Stops the Companion and the window the same way, removes the files, shortcuts, the uninstall entry, the Run value (only when it points at this install) and the startup marker, so a later install turns startup at sign-in back on. It **keeps user data** unless you answer Yes when it asks, or pass `--delete-app-data` to a silent uninstall. The prompt reads "Also delete your Open TouchDeck Layouts and settings, including Integration settings, from this PC?" and adds that Integration secrets such as API keys stay in Windows Credential Manager; Forget in Open TouchDeck removes them first. Integration secrets stay either way.

Installers are not Authenticode-signed: there's no certificate, so Windows SmartScreen may warn. The update signature is separate from Authenticode and is always checked.

## Update signing

The update key is the existing updater key ([ADR 0005](adr/0005-updater-keeps-the-signing-key.md)). A separate release signing job signs the installer with the repo's own signer, `.github/release-signer/cli.mjs sign --app-version X.Y.Z`. It writes the same minisign signature the Tauri CLI 2.12.4 `tauri signer sign --app-version` wrote: Ed25519 over the installer's BLAKE2b-512 hash, with the file name and version bound in its trusted comment, which the Electron updater verifies. It reads the key from `TAURI_SIGNING_PRIVATE_KEY` (or a file named by `--private-key-path` or `TAURI_SIGNING_PRIVATE_KEY_PATH`) and its password from `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, the names the Tauri CLI used. It needs only Node.js, with no packages. Never regenerate or replace the key: every installed copy trusts it.

Set `TAURI_UPDATER_PUBLIC_KEY` as a repository Actions variable so the unsigned build can read it without the `release` environment. Keep `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` as secrets only in the `release` GitHub environment. Copy only the public variable from the environment to repository scope when enabling this workflow. Only `main` and `v*` tags can deploy to it.

A release contains:

```text
OpenTouchDeck-windows-x64-setup.exe
OpenTouchDeck-windows-x64-setup.exe.sig
latest.json
release-manifest.json
touchdeck.tdfw
touchdeck-radio-X.Y.Z.tdfw   (only when the release ships the radio build)
SHA256SUMS
```

`touchdeck.tdfw` is the normal, radio-free build the installer carries. `touchdeck-radio-X.Y.Z.tdfw` is the opt-in radio build, a separate file. The sign and publish jobs refuse a release that:

- has no Firmware record, or whose installer Firmware is the radio build
- ships the radio build but lacks the radio file, or whose radio file is the normal build
- ships no radio build but holds a radio file or record
- has a bundle whose hash differs from `release-manifest.json`

### The radio build in a release

`app/release-options.json` decides whether a release builds and ships the radio build:

```json
{ "radio_build": false }
```

With `false`, the recipe builds only the normal Firmware, the Companion carries no radio build, and Device settings shows no build switch to a Device on the normal build. A Device already on the radio build still sees its build and can switch back if the Companion carries the normal build. The sign job reads the file at the tagged commit and refuses assets that disagree with it. Set it to `true` once the radio build is ready to ship. Releases leave it out until the Wi-Fi link's open security work is done (#318).

What protects each Firmware bundle:

- **Neither `.tdfw` has a signature of its own.** Both builds reach a PC inside `td-companion.exe`, and the Companion installs only those embedded bundles or a file someone picks by hand.
- **Embedding checks the hash.** The recipe records each bundle's SHA-256 in `release-manifest.json` and passes it to `build.rs` (`TOUCHDECK_FIRMWARE_BUNDLE_SHA256`, `TOUCHDECK_RADIO_FIRMWARE_BUNDLE_SHA256`). `build.rs` refuses to embed a file with any other hash.
- **The installer has the update signature (ADR 0005).** The updater checks it before it runs an update. The installer is not Authenticode-signed, and a first install from a download is checked only against `SHA256SUMS`.
- **The Companion checks a bundle before writing it.** Every file must match the SHA-256 in the bundle's own `manifest.json`, and `touchdeck.bin` must match `elf_sha256`. This catches a damaged file, not a forged one.
- **The loose release files are covered only by `SHA256SUMS` and `release-manifest.json`.** The release jobs check both `.tdfw` files against them. A file picked by hand gets the per-file check above and nothing more.

`latest.json` holds the version, notes, date and, for `windows-x86_64`, the installer URL, its signature and its size. Installed apps read `https://github.com/Duresa7/OpenTouchDeck/releases/latest/download/latest.json`.

## The bridge to 2.0

Open TouchDeck 2.0 is a native app that ships as a per-user MSI for Windows 11 only ([ADR 0011](https://github.com/Duresa7/OpenTouchDeck/blob/main/docs/adr/0011-native-windows-app-in-rust-and-slint.md)). Its source and release pipeline are on `main`. 1.x moves to it in two steps: 1.x copies update to this release, 1.3.0, through `latest.json` as usual, and 1.3.0 then offers 2.0.

**What 1.3.0 checks.** The updater checks `latest.json` first, so a newer 1.x, such as a fix to the bridge itself, always comes before 2.0. When 1.x is up to date, it reads the repository's release list from `api.github.com` with the same rules as 2.0's own updater: the newest release whose tag is a stable `v2.x.y`, not a draft or pre-release, with exactly one `OpenTouchDeck-X.Y.Z-windows-x64.msi` and one `.msi.sig` at that release's own download URLs. The request carries the app's name as its User-Agent and nothing about the PC. A failed lookup never hides the 1.x result.

- **Windows 11** (build 22000 or later) is offered 2.0. The dialog says it replaces the app and that Layouts, settings, Devices and Pairings stay.
- **Windows 10** is never offered 2.0. Settings → About says 2.0 needs Windows 11 and that this PC keeps 1.x, which still gets updates. The 2.0 MSI refuses Windows 10 too.

**Verifying the MSI.** The download checks the MSI against the `.sig` with the updater key ([ADR 0005](adr/0005-updater-keeps-the-signing-key.md)), and the signature's trusted comment must name this file and this version. Right before the hand-off it checks again, in one pass that also takes the file's SHA-256.

**The hand-off.** The updater takes `system.lock` and stops the Companion through its shutdown route, as for any update. It copies `resources\bridge.ps1` next to the verified MSI in its staging folder, starts it with the MSI's SHA-256, and quits. `bridge.ps1`:

1. Refuses, changing nothing, unless the PC runs Windows 11, 1.x is installed in `%LOCALAPPDATA%\Programs\Open TouchDeck`, the MSI still has the SHA-256 the signature check read and 1.x has exited. It waits for 1.x to exit, then ends any 1.x process still running from the install folder: the Companion already stopped, so nothing is lost.
2. Moves the 1.x Start menu shortcut into its bridge folder and renames the install folder to `Open TouchDeck.1x-backup`. The MSI installs to the same folder and refuses to run beside 1.x.
3. Runs the MSI with Windows Installer's progress bar (`msiexec /i ... /passive /norestart`), which has a Cancel button.
4. If the MSI fails or is cancelled, renames the folder back and restores the shortcut, and 1.x starts again exactly as it was. At its next start 1.x says the update to 2.0 didn't happen and that Layouts and settings are still there.
5. If the MSI succeeds, removes what 1.x left, each item only when it points at 1.x: its uninstall entry, its install record under `HKCU\Software`, its Start menu and desktop shortcuts, its Run value, the Electron window's profile (`%APPDATA%\Open TouchDeck`, which holds no user data because the window's session lives in memory) and the renamed folder. It then starts 2.0 with `--packaged` and removes the staging folder.

It never runs the 1.x uninstaller: that one always removes `%LOCALAPPDATA%\Programs\Open TouchDeck` and the Start menu shortcut named Open TouchDeck, which are the 2.0 MSI's folder and shortcut too.

**User data stays where it is.** 2.0 reads the same app data folders and the same Credential Manager entries as 1.x; `bridge.ps1` never touches them. [`docs/packaging.md` on `main`](https://github.com/Duresa7/OpenTouchDeck/blob/main/docs/packaging.md) maps each kind of data. Start at sign-in carries over: 1.x and 2.0 share the Run value's name and the `startup-initialized` marker, so when 1.x started at sign-in the bridge removes its value and the marker and 2.0 turns start at sign-in on at its first start. When 1.x had it off, the marker stays and 2.0 leaves it off.

**Where it reports.** `bridge.ps1` writes `bridge.log` and `result.json` to `%LOCALAPPDATA%\io.github.duresa7.touchdeck\bridge`, plus `msi.log` when the MSI failed. Its exit code is 0 when 2.0 is installed, 1 when it refused and changed nothing, 2 when the MSI failed or was cancelled and 1.x is back, and 3 when 1.x could not be put back; the log then says where it is.

**What it can't undo.** A power cut between the rename and the end of the MSI leaves 1.x in `Open TouchDeck.1x-backup` with no shortcut. User data is untouched; installing 1.x or 2.0 again fixes it. A taskbar pin of 1.x points at the old program afterwards and needs pinning again.

**Proof.** The **bridge upgrade** workflow on `main` installs a 1.x build from this branch, seeds user data, refuses a tampered MSI, rolls back from an MSI that fails, then moves to a 2.0 MSI built from `main` and checks that 2.0 finds the data and that nothing of 1.x is left.

**Releasing it.** 1.3.0 is published as the latest release like any 1.x. Once a 2.x release is the latest, `releases/latest/download/latest.json` is the 2.x release's. A 2.x release therefore carries a copy of the newest 1.x `latest.json`, so 1.x copies that never reached 1.3.0 still find the bridge. A later 1.x fix for Windows 10 is published without "Set as the latest release".

## Shipping an update

1. Bump every app version in one command (Python 3.11+), open a pull request with the result and merge it. `check` fails when `app/package.json` and `app/electron/package.json` disagree.

   ```sh
   python app/scripts/release_version.py bump X.Y.Z
   python app/scripts/release_version.py check
   ```

2. Tag the merged commit on `release/1.x` with an annotated tag. Its message becomes the release notes. 1.x releases come only from `release/1.x` ([ADR 0012](https://github.com/Duresa7/OpenTouchDeck/blob/main/docs/adr/0012-electron-and-react-leave-main-before-cutover.md)); the build job refuses a tag on any other branch.

   ```sh
   git switch release/1.x
   git pull --ff-only
   git tag -a vX.Y.Z -m "<notes>"
   git push origin vX.Y.Z
   ```

3. The tag push runs **windows release** (`.github/workflows/release-windows.yml`). It runs the app, Firmware and installer lifecycle checks on the tagged commit without secrets, builds without secrets, signs on a separate runner, and verifies the signed installer through the real updater against local HTTPS fixtures. The draft job uploads every asset, checks the uploaded list and sizes, then stops.

4. Review the draft and its assets on the Releases page, then publish it. Publish it as the latest release only while no 2.x release is published (see [The bridge to 2.0](#the-bridge-to-20)). The workflow never publishes it automatically.

To rehearse without publishing, run **windows release** by hand on a commit on `release/1.x`. It builds and signs with the durable key and keeps the assets as a workflow artifact. The `release` environment must allow the `release/1.x` branch for this; a tag push needs only its `v*` tag rule:

```sh
gh workflow run release-windows.yml --ref release/1.x -f ref=release/1.x
```

If a tag's run fails for a transient reason, re-run it; the draft job reuses its own draft. If the source needs a fix, ship the next patch version instead of moving the tag.

## Installer lifecycle check

`installer-lifecycle.yml` runs on a disposable GitHub-hosted Windows runner as part of every **windows release** run. Run it by hand to check an installer change on a branch:

```sh
gh workflow run installer-lifecycle.yml --ref <branch>
```

It builds a test-signed candidate with a throwaway key that is deleted after signing, plus a second installer at the next patch version signed with the same throwaway key. Then `app/scripts/installer_lifecycle.ps1`:

1. installs silently and opens the app: the window and the Companion start, and the Companion registers the Run value;
2. runs the real Electron updater (`app/scripts/lifecycle_update.mjs`, connected to the installed Companion as the window is) against a loopback feed that serves the newer installer with one byte changed, and expects a signature failure with nothing installed;
3. updates to the newer installer through the same updater: lock, shutdown, `/S --updated --force-run`, restart, user data kept byte for byte;
4. runs the older installer over the newer one and expects it to refuse;
5. uninstalls silently with the Companion running: its runtime file is gone (it shut down cleanly), and the files, uninstall entry and Run value are gone while user data stays;
6. reinstalls and uninstalls with `--delete-app-data`, and expects the data folders gone.

The script refuses to run outside GitHub Actions: it writes the real per-user registry. It uploads only a sanitized log and summary.

## Build the installer

Use Windows x64 with Rust (MSVC), Node 24, the pinned pnpm, Python 3.11+ and ESP-IDF 6.1 with ESP32-S3 tools, and activate ESP-IDF. Build a clean committed revision into a new folder outside the checkout:

```powershell
Set-Location app
pnpm install --frozen-lockfile
python scripts/package_windows.py build --out E:/touchdeck-artifacts/build-001 `
  --public-key-file E:/secure-public-material/touchdeck-updater.pub `
  --notes-file E:/touchdeck-artifacts/release-notes.md
```

The recipe builds the Firmware and audits it radio-free, then, when `app/release-options.json` ships it, builds the radio build in `build/firmware-radio` and holds it to `radio_guard.py radio` and the static internal RAM ceiling. It builds the UI, builds the Companion with `cargo build --release --locked -p td-companion` and both bundles embedded (`TOUCHDECK_FIRMWARE_BUNDLE` and `TOUCHDECK_RADIO_FIRMWARE_BUNDLE`, each pinned to its recorded SHA-256), keeps the radio bundle out of the installer payload, refuses a Companion with test hooks, builds the Electron code with the updater public key, packages it with electron-builder. Durable releases leave the signature empty until the separate signing job; use the workflow to produce verified release assets.

For an unpublished candidate, pass `--test-signing` instead of `--public-key-file`, with no signing environment. It generates a throwaway key, builds the installer named `OpenTouchDeck-windows-x64-TEST-ONLY-setup.exe` and the next-patch installer for the lifecycle check, and deletes the private key. Never publish these assets. On a PC without ESP-IDF, a test-signed candidate can reuse a checked bundle with `--firmware-bundle <file.tdfw>`, and the radio build with `--radio-firmware-bundle <file.tdfw>` next to it. Each refuses the other build, and without `--radio-firmware-bundle` the candidate carries the normal build alone. Releases build the radio build only when `app/release-options.json` says so.

The recipe holds the shared `.verify/m3-build.lock` while it compiles both Firmware builds and removes only its own lock. Never delete another owner's lock.

The installer enforces current-user mode and `%LOCALAPPDATA%\Programs\Open TouchDeck` after resolving command-line and stored installation settings. `/allusers` and `/D=` cannot change that destination. A busy Device stops the installer with exit code 3. If the Companion passes authentication proof but the helper cannot confirm an install lock, the installer exits 4 and asks you to close Open TouchDeck from the tray before retrying.

## Signing boundary and dependency disposition

The job graph is `build -> sign -> verify -> publish` (the last job only prepares a draft).
The app, Firmware and installer lifecycle checks must also pass before that draft is prepared.
Build and verification have read-only repository tokens and no signing secrets. Signing has a
read-only token and the `release` environment; only its signer invocation receives the secrets.
It checks the installer SHA256 against the build job output before signing. It checks out only
`.github/release-signer`, two dependency-free Node.js files that use `node:crypto`, and installs
nothing: no npm package, app dependency or lifecycle script runs on that runner. The signature is
uploaded separately. Verification checks the hash again, accepts the signed bytes and rejects
a tampered copy through the real updater, and uploads no release files. Verification checks
out the exact commit resolved by build and
uses the repository public-key variable, refusing a different key in the manifest. The draft
job downloads the original build and signature artifacts, checks the build hash, and creates
the feed and checksums itself using the annotated tag notes from GitHub. Signing keeps its
signer at the workflow commit, fetches the built commit only to read its package
version, and checks that version, the tag version and production signing mode before signing.
Only the draft job has a write token. The Python recipe also removes every `TAURI_SIGNING_PRIVATE_KEY*`
variable, case-insensitively, from build subprocess environments, including inherited ones.
It also refuses ambient signing variables in either build mode before importing any Firmware tooling.

As of the security fix for #108, electron-builder 26.18.0 still resolves minimatch versions
covered by GHSA-g3hv-75wg-m249. The compatible update does not fix the advisory; no major
minimatch override is applied. Exposure is in development packaging dependencies, where
repository-controlled file patterns are processed. Track an upstream compatible fix before
accepting untrusted build patterns. The audit remains nonzero.

RUSTSEC-2026-0344 remains in ruzstd 0.8.3 via object 0.39.1 and espflash 4.5.0.
`cargo update -p espflash` finds no newer version compatible with the workspace's Rust 1.88
requirement; espflash 4.7.0 requires Rust 1.95. The Companion recovery path in
`app/crates/td-link/src/recover.rs` constructs `Segment`s from bundle image bytes and calls
`Flasher::write_bins_to_flash`; it does not call the ELF/debug-symbol loader or object section
decompression. espflash's CLI feature is disabled. The vulnerable zstd decoder is present
transitively but no attacker-supplied zstd decoding path was found in td-link. Revisit on any
ELF/debug-symbol support or compatible upstream update; the audit remains nonzero.
