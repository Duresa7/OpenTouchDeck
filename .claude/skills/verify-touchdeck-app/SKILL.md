---
name: verify-touchdeck-app
description: Build and verify the Electron app, Rust td-companion and React UI with isolated mock Devices, authenticated API calls, Playwright specs and screenshots. Use to prove app behavior or identify the Companion build under test.
---

# Verify the Open TouchDeck app

The Rust Companion in `app/crates/td-companion` owns Devices, actions, Integrations and OS access. It serves the authenticated API and the embedded React UI from `app/ui/dist`. The thin Electron window in `app/electron` is an API client. React Playwright specs live in `app/ui/e2e`; fake mode uses demo data, while Companion mode uses a real debug Companion with mock Devices. The CLI in `app/crates/td-link` is a separate USB client.

Run from PowerShell. `$TA` means `pwsh -NoProfile -File .claude/skills/verify-touchdeck-app/scripts/tdapp.ps1`, run from the repo root. It works on the checkout it lives in; set `$env:TOUCHDECK_ROOT` to test another one. Stdout is one JSON result, chatter is stderr. Node 22+ (built-in WebSocket) and the repository's pinned pnpm and Rust must be on PATH. Pass absolute data and evidence paths.

## Commands

| Command | Behavior |
| --- | --- |
| `$TA build [--firmware-bundle <abs .tdfw>]` | Frozen dependency install, `pnpm build:ui`, debug `cargo build -p td-companion --features test-hooks`, debug `cargo build -p td-link`. `--firmware-bundle` sets `TOUCHDECK_FIRMWARE_BUNDLE` for the Companion build, so a real Companion offers that bundle; see [board pass tools](features/board-pass-tools.md). |
| `$TA doctor [--out <json>]` | Read `/api/v1/system` build SHA against checkout HEAD: exit 0 match, 1 differ, 2 missing/unusable. If stopped, launch a temporary isolated mock Companion and stop it afterward. |
| `$TA start [--mock] [--device-console] [--port <n>] [--data <dir>] [--log <file>]` | Start the debug Companion, wait for authenticated API readiness, record PID, executable and process creation time in `app/target/tdapp-state.json`. Mock supplies three Devices. `--device-console` (real Companion only) passes `--test-real-device-console`, so `devcmd` reaches the real Device. Default API port 50744; default data `app/target/tdapp-data`. The Companion's stdout and stderr are appended to `--log`, default `<data>/companion.log`, and the result's `log` names it. |
| `$TA stop` | Graceful API shutdown, with process termination fallback only for the recorded PID whose path and creation time still match. Reports remaining owned processes. Idempotent. |
| `$TA api <METHOD> <path> [--body <json>]` | Call the running Companion, with `/devices` or `/api/v1/devices` as path. Result has HTTP status and body. |
| `$TA devcmd "<line>" [--device <id>] [--wait-ms <n>]` | Send one Firmware console line through the running Companion's `test.console` and print `reply` (the `@td` JSON whose `cmd` is the line's first word, or null) and `lines` (console lines up to the reply, at most 256). Waits up to `--wait-ms`, default 3000, at most 15000. Without `--device`, needs exactly one connected Device. Exit 0 reply `ok`, 1 reply not ok or none in time, 2 refused. Mock Companion, or real one started with `--device-console`. See [device console](features/device-console.md). |
| `$TA events --seconds <n>` | Capture events directly through Node: hello, sync and subsequent state frames for up to 300 seconds. |
| `$TA ui-e2e <spec> [--companion] [--port <n>] --out <dir>` | One exact basename from `app/ui/e2e`, fake mode by default. Companion specs require `--companion`. Writes `report.json`, `results/` screenshots, failure traces and optional TD_SHOTS captures. Owns and tears down its temporary servers. |
| `$TA screenshot --out <png>` | Headless Edge screenshot of the running Companion's embedded React UI using a verification window-session bridge. Captures once every connected Device's Home card has drawn its Layout miniature or "No Layout yet"; fails after 15 s otherwise. |
| `$TA link <td-link args>` | Debug CLI passthrough, collected as one JSON result with a results array; retains CLI exit code. Example: `link --mock ping --count 10`. |
| `$TA test` | `pnpm test`, then `cargo test --workspace`, with both exit codes. |

API operations verify the runtime file's HMAC proof before minting a window verification session, and revoke it on completion. Session and pairing tokens stay out of the command's JSON. The runtime file under the data directory remains private local material.

## Isolation and real Devices

Use distinct API ports and data folders for mock lanes in different checkouts. One recorded start per checkout; use another checkout for a second started Companion. Companion ports are 49152..65535. Fake e2e defaults to UI port 5184. Companion e2e defaults to 50844 (UI) and 50845 (Companion); `--port` selects the UI port and reserves the next port for its Companion. Servers refuse occupied ports rather than attaching to another lane. Use fresh evidence directories.

Real Device work requires ownership of the hardware lock: `.verify/hardware.lock` in the main checkout, which every worktree shares (override the path with `$env:TOUCHDECK_HARDWARE_LOCK`). The lock records the holder and time. The helper requires its presence; the operator/agent must establish ownership. Follow the board-pass standing orders before any hardware access. `$TD` is the PowerShell Firmware wrapper in the sibling `verify-touchdeck` skill. Run `$TA stop` before any `$TD serial` call or a real `link` call. While the real Companion owns the port, send single-reply console commands (`touch`, `ui`, `keys`, `power`, `perf`, `layout state`) with `$TA start --device-console` and `$TA devcmd` instead; streaming commands (`screenshot`, `keys record dump`, `store put|get`) still need `$TD` with the Companion stopped. The Companion discovers ports automatically; `start --port` selects the HTTP API, not a COM port. Record Firmware Doctor identity alongside Companion Doctor identity. Mock proof never establishes physical USB, Firmware rendering, OS key injection or native confirmation behavior.

## Evidence and feature routing

Save command JSON, stderr logs, screenshot PNGs and e2e output in `.verify/<task>/lane-<n>/`. [The feature map](features/README.md) links every current React spec and Companion API feature. Read the relevant file before selecting a spec. [Board pass tools](features/board-pass-tools.md) covers the Companion log, staging a Firmware bundle newer than the Device, the console flood, and which board-pass steps each unblocks. Return paths and actual exit codes, plus skipped behavior.

## Gotchas

- Build the UI before Rust: the Companion embeds it at compile time. A successful Rust build alone can still serve the UI-not-built placeholder.
- `--mock-devices` and `--test-real-device-console` need `test-hooks`; this debug verification build is unsuitable for distribution. A build without `test-hooks` rejects both as `Unknown Companion argument.`, and `package_windows.py` refuses a binary that contains either.
- Doctor compares HEAD, not uncommitted source changes. With no usable build identity it fails instead of inferring from timestamps or build records.
- The Companion exposes no standalone build-info CLI. Doctor starts a temporary mock to query a stopped binary. An ordinary binary without test-hooks can be checked only while already running through this helper's state.
- A screenshot proves the served UI, not Electron preload isolation, the native frame, focus or desktop feel. Electron-specific checks are `pnpm --filter @touchdeck/electron e2e` after the debug build; these are separate from `ui-e2e`.
- Events use the bearer subprotocol directly from Node. A plain browser page redirects to `/pair` without a session bridge, destroying page-based captures.
- The helper uses installed Edge through repo Playwright, so no browser download is needed for these proof lanes. Pairing specs also need Chromium installed via repo Playwright.
- Broken pnpm (empty version or app-picker dialog) is a coordinator repair; leave global pnpm unchanged.
- Keep build output on E:, retain the user's Cargo job cap, and use debug builds. Run the workspace test command once for a full gate.
- The Windows npm shim may point to `node_modules/pnpm/pnpm.exe` rather than putting that executable on PATH. The helper resolves it without changing the global installation.
- Playwright may log a NO_COLOR/FORCE_COLOR environment warning, and mock Devices may log identity fallback diagnostics. These are runtime diagnostics; the debug build must still complete without compiler warnings.
