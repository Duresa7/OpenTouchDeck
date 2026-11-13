---
name: verify-touchdeck-app
description: Verify the 2.0 Rust and Slint app on main with mock Devices, headless screen and model tests, native snapshots and td-link. Use to prove app behaviour or identify the build under test on Windows or Linux.
---

# Verify the 2.0 app

`app/crates/td-app` runs the Companion and Slint window in one process. Read the
[feature map](features/README.md) before choosing evidence. It distinguishes
implemented screens from models awaiting a Slint page. Use typed Companion
sessions and Rust tests for behaviour, and the mock app for rendered evidence.

## Build and identify

Use the pinned Rust toolchain and platform dependencies in `docs/setup.md`.
The helper needs Python 3.11 or newer, with `python` on PATH. Commands below
start at the repository root unless a block changes directory. Keep evidence
under a git-ignored `.verify/<task>/` directory; use a fresh `--out` for each run.

Build in the foreground, then identify the exact debug executable:

```sh
cd app
cargo build --locked -p td-app --features mock
cd ..
python .agents/skills/verify-touchdeck-app/scripts/tdapp.py identity
```

The identity JSON records checkout HEAD, dirty state, Rust and Cargo versions,
package version, expected profile/features and the executable's SHA-256. It
resolves the target directory through Cargo metadata, including a custom
`CARGO_TARGET_DIR`. Build immediately before collecting identity and keep the
source unchanged during the run. This is local build provenance, not an embedded
commit check. About currently shows the Cargo package version, which can still
be `1.2.0` in the 2.0 app. It cannot identify a commit or a running installed copy.

## Run and capture

```sh
python .agents/skills/verify-touchdeck-app/scripts/tdapp.py run --out .verify/app-2/window
python .agents/skills/verify-touchdeck-app/scripts/tdapp.py screenshot --page settings --out .verify/app-2/settings
```

`run` executes `cargo run -p td-app --features mock -- --mock-devices 2` from
`app/`, with build output attached to the terminal. Inspect Home for two mock
Devices, then use Ctrl+C to stop. On Linux, closing the window also exits; on
Windows, closing hides it in the tray, so use Quit or Ctrl+C.

Both helper commands isolate app data and source configuration in a temporary
directory, set `SLINT_BACKEND=winit-software`, and write `build.json` and
`result.json` under `--out`. They stop their owned process on completion or
interruption and remove temporary data. A forced termination is reported in
`cleanup`; treat it separately from a graceful app exit. The helper never
attaches to another app instance.

`screenshot` launches the prebuilt mock executable, requests a PNG through the
app's existing `TOUCHDECK_SNAPSHOT_DIR/request` hook, waits for the complete file,
and stops the app. Exit 0 means a complete PNG was captured; exit 2 means the
command failed. Inspect `snapshots/screen.png`, `app.log` and the dimensions in
`result.json`; a PNG alone does not establish the expected state.

Choose `--page home`, `setup`, `settings`, `integrations`, `about`, `device:mock0001` or
`integration:<id>` (`cliproxyapi`, `custom`, `icon-packs`, ...).
`--mock` accepts comma-separated `device`, `radio`, `v1`, `board`, `fresh` kinds;
its default is `device,device`. Use `radio` for paired apps and `fresh` for Setup
following Firmware install. `--settle-seconds` delays the request, default 3;
increase it for asynchronous install/discovery. `--timeout` bounds the whole
capture wait, default 30 seconds. Inspect the state actually captured and use a
new output directory for each page or mock combination.

Rendering needs a desktop session on Windows or X11/Wayland on Linux. A Linux
agent without a desktop can run the screenshot command under an available
Xvfb session. The app renders its contents off the desktop through Slint's
snapshot API, but still creates a window. The headless testing backend below
does not render pixels. These captures prove app content, not DWM Mica, native
frame, OS focus, tray behaviour or physical Device rendering.

## Headless behaviour and checks

From `app/`, these work on Windows and Linux without a display. Use debug tests
because release builds omit element IDs that the UI tests search.

```sh
cd app
cargo test --locked -p td-app --lib screens_tests::
cargo test --locked -p td-app --lib ui_tests::
cargo test --locked -p td-companion --features test-hooks --test editor_session --test integrations_session --test integration_setup
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cd ..
```

M2's `screens_tests` use `i_slint_backend_testing::init_no_event_loop` through
`ui_tests::init`. They invoke controls by accessible role and label against an
in-process Companion with mock Devices. They prove Setup, Settings and Pairings
without opening a window. The session tests prove the Layout editor and
Integrations models through typed interfaces. The workspace run covers sibling
crates too. Save actual test counts and failures; a filtered run with zero tests
is not evidence. `mock` and `test-hooks` builds are local verification builds.

## Drive td-link

From `app/`, these exercise the separate link CLI against its own mock Device:

```sh
cd app
cargo run --locked -p td-link --bin td-link -- --mock hello
cargo run --locked -p td-link --bin td-link -- --mock ping --count 100
cargo run --locked -p td-link --bin td-link -- --mock settings get
cd ..
```

Require the expected JSON, including all 100 matching ping replies. Exit codes
are 0 for success, 1 for a Device refusal and 2 for other failures. Each CLI
invocation creates a fresh mock; its changes do not affect the mock app.

For real USB work, close the app connection and serial monitors first, select
the Device's port locally with the CLI's `--port` option, and follow the sibling
`verify-touchdeck` skill's hardware ownership and Firmware identity procedure.
Keep port names and raw Device identity in ignored evidence. Mock runs establish
neither USB operation nor real OS key injection.

## Windows desktop evidence

For native Windows look, accessibility tree, tray and memory, reuse
`.github/workflows/windows-evidence.yml` and
`.github/scripts/windows-evidence/README.md`. The suite captures Home and About
through desktop screen copying; the helper above uses the app's existing content
snapshot hook for other screens. There is no second desktop capture system.

Only on a disposable Windows desktop, build from `app/`, then run the existing
suite from the repository root in Windows PowerShell 5.1:

```powershell
cd app
cargo build --release --locked --package td-app --features mock
cd ..
powershell -ExecutionPolicy Bypass -File .github/scripts/windows-evidence/run.ps1 -Exe app/target/release/td-app.exe -Out .verify/app-2/windows
```

The suite changes theme, contrast, text scale and Start at sign-in, restarts
Explorer, and leaves those changes in place. It writes `summary.md`,
`results.json`, logs, UI Automation trees and `screenshots/`. Report each failed
or unavailable check. GitHub's Windows Server runner records its OS/build;
its flat Mica fallback does not prove Windows 11 GPU appearance. The suite uses
the default Cargo target directory; adjust `-Exe` if yours differs.

## Report and clean up

Report checkout HEAD and dirty state, executable digest, platform, exact commands,
exit codes and test counts, screenshot paths, observed behaviour and remaining
platform or hardware checks. Keep raw logs local and cite repo-relative evidence
paths publicly. Require the app and test processes you started to be stopped.
List Windows-only checks as pending when working on Linux.

## 1.x verification

Verify the Electron and React app from `release/1.x`, using that branch's copy of
`verify-touchdeck-app`. Its HTTP, Playwright and Node tooling belongs there.
