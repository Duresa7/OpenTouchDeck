# Open TouchDeck desktop app

Open TouchDeck runs on Windows and Linux. It connects to the deck over USB-Serial/JTAG,
edits and transfers layouts, runs PC actions, feeds CLIProxyAPI usage widgets, and updates deck
firmware. The running, connected app mirrors keys and media controls into the host
session. The deck does not type as a standalone USB HID keyboard. Bluetooth is not
supported. Wi-Fi, battery support, and standalone USB HID are deferred to the post-product update.

For everyday use, follow the [USB user guide](../docs/usage.md). **Settings > Troubleshooting > App build**
identifies the app's source revision. The window title reads **Open TouchDeck**. **Deck** reports the connected firmware's
version and image identity. Desktop app updates and deck firmware updates are
separate operations.

Built with Electron, React, and a Rust Companion, the app discovers USB decks and provides a debug console pane.

## Install a published build

The Windows installer is available as
[OpenTouchDeck-windows-x64-setup.exe](https://github.com/Duresa7/OpenTouchDeck/releases/latest/download/OpenTouchDeck-windows-x64-setup.exe)
on [the latest GitHub release](https://github.com/Duresa7/OpenTouchDeck/releases/latest).
It installs Open TouchDeck (the Electron window, the Companion and the matching
Firmware bundle) per user under `%LOCALAPPDATA%\Programs\Open TouchDeck` and needs
no admin rights, Node, pnpm, Rust, or ESP-IDF. Linux has no installer.

The [desktop setup guide](../docs/setup.md) covers prerequisites and Linux serial,
`/dev/uinput`, and credential-store access. The
[packaging guide](../docs/packaging.md) describes the installer's contents,
candidate artifacts, and release production. A successful source build or
candidate package is not a published release.

The installed app updates itself from GitHub releases signed with the
project's updater key; the installer is not Authenticode-signed. Builds without
`TOUCHDECK_UPDATER_PUBKEY` carry no updater key and report updates as disabled.
To build the installer, run `python scripts/package_windows.py build` as the
packaging guide describes.

The Companion embeds `ui/dist` at compile time: run `pnpm build:ui` before building Rust. Without that directory it serves a small "UI not built" page.

## Build the desktop app

Use Node.js **24**, pnpm **12.12.1** as pinned in [package.json](package.json),
and the Rust version pinned in [rust-toolchain.toml](rust-toolchain.toml), which
rustup installs for you. Follow the
[desktop setup guide](../docs/setup.md) for platform dependencies.

On Windows, from the checkout root, the setup command is:

```powershell
pwsh -NoProfile -File app/scripts/setup-windows.ps1
```

The script installs missing development tools. Review it before running it, and
open a new PowerShell afterward. It is a developer setup script, not the desktop
app installer. Linux builds also need the libudev development package described in the setup guide.

From `app/`, build the production app:

```sh
pnpm install --frozen-lockfile
pnpm build:ui
cargo build -p td-companion
pnpm --filter @touchdeck/electron build
```

The Companion executable is `target/debug/td-companion.exe` on Windows or
`target/debug/td-companion` on Linux. The Companion embeds the built UI.
The packaging recipe owns installer and release generation.

## Develop and check changes

Run these commands from `app/` after installing dependencies:

```sh
pnpm dev:electron
pnpm lint
pnpm test
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

To run the Linux app checks on the committed `HEAD` in Docker (needs Docker; the first run takes about 5 minutes and later runs take about 3 minutes):

```sh
./scripts/linux-check.sh
```

### Electron shell

`electron/` is the thin Electron window that loads the UI the Companion serves.
`pnpm dev:electron` builds the Companion with `--features test-hooks`, starts it
with one mock Device in a scratch data directory, starts the UI dev server
(`ui/`), and opens the window on it. Unpackaged builds load the UI dev server
when `TOUCHDECK_UI_DEV_URL` names it (`http://127.0.0.1:PORT/` only); the
window session is still handed only to the Companion's own origin. The
window's end-to-end tests need that build too:

```sh
cargo build -p td-companion --features test-hooks
pnpm --filter @touchdeck/electron e2e
```

The shell starts the Companion from `TOUCHDECK_COMPANION_BIN` in development and
tests, and from `td-companion` next to the app executable when packaged.

## Repository layout

| Path | Responsibility |
| --- | --- |
| `crates/td-core` | Frame codec, link messages, layout validation, settings, and source-config path/storage helpers. |
| `crates/td-link` | Serial and mock transports, transfers, firmware update and ROM recovery, and the link CLI. |
| `crates/td-host` | Host input, actions, approvals, icon processing, and Sources. |
| `crates/td-companion` | Background Companion and local API. |
| `electron` | Desktop window and updater. |
| `ui` | React UI and Playwright specs. |

The [layout format](../schema/README.md) and
[framed app link](../schema/protocol/link-v1.md) describe the files and protocol.

## Use the link CLI

`td-link` is another USB client. Close the desktop app's connection and any serial
monitor before using it against the same port. These commands run from `app/`:

```sh
cargo run --release -p td-link --bin td-link -- discover
cargo run --release -p td-link --bin td-link -- --port COM3 hello
cargo run --release -p td-link --bin td-link -- --mock ping --count 1000
cargo run --release -p td-link --bin td-link -- --port COM3 console "version"
cargo run --release -p td-link --bin td-link -- --port COM3 events --seconds 10
```

Replace `COM3` with your deck's port. The CLI prints JSON and exits `0` on success,
`1` when the deck replies with `ok` false, and `2` on another failure.

## Run end-to-end tests

Run the React specs against the fake Companion from `app/`:

```sh
pnpm e2e:ui
```

CI splits them into four parallel shards. Run one shard locally with `pnpm e2e:ui --shard=1/4`.

For the real Companion API tests:

```sh
pnpm build:ui
pnpm e2e:ui:companion
```

UI package boundaries and entry-point conventions: [UI packages](ui/src/lib/README.md).
