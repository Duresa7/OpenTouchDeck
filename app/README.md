# Open TouchDeck desktop app

This folder is the Rust workspace for Open TouchDeck 2.0: one program that runs the Companion and a native
Slint window ([ADR 0011](../docs/adr/0011-native-windows-app-in-rust-and-slint.md)). It talks
to the Device over USB-Serial/JTAG, edits and transfers Layouts, runs PC actions, feeds Integrations and
updates Firmware.

**The app people install today is 1.x**, an Electron window and React UI in front of the same Companion. Its
source is not on `main`: it lives on the [`release/1.x` branch](https://github.com/Duresa7/OpenTouchDeck/tree/release/1.x/app), where 1.x fixes and the release
that moves 1.x to 2.0 are built
([ADR 0012](../docs/adr/0012-electron-and-react-leave-main-before-cutover.md)). To build or change the 1.x
app, check out that branch and follow its copy of this page. For everyday use, follow the
[USB user guide](../docs/usage.md).

The window is `td-app`, described under [Native app](#native-app-20-in-development). The `td-companion`
program alone has no window: it runs the Devices, the tray and the local JSON API
([contract](../docs/companion-api.md)), and its tests drive it through that API.

## Build

Use the Rust version pinned in [rust-toolchain.toml](rust-toolchain.toml), which rustup installs for you.
Nothing here needs Node or pnpm. Follow the [desktop setup guide](../docs/setup.md) for platform
dependencies.

On Windows, from the checkout root, the setup command is:

```powershell
pwsh -NoProfile -File app/scripts/setup-windows.ps1
```

The script installs missing development tools: rustup and the MSVC C++ Build Tools. Review it before running
it, and open a new PowerShell afterward. It is a developer setup script, not the app's installer. Linux
builds need the libudev development package described in the setup guide.

From `app/`:

```sh
cargo build -p td-companion
```

The Companion executable is `target/debug/td-companion.exe` on Windows or `target/debug/td-companion` on
Linux. The [packaging guide](../docs/packaging.md) describes the 2.0 MSI and its release pipeline.

## Native app (2.0, in development)

`crates/td-app` is the Open TouchDeck 2.0 program from
[ADR 0011](../docs/adr/0011-native-windows-app-in-rust-and-slint.md): the Companion and a
Slint window in one process. It is not released yet; the 1.x app on `release/1.x` is what people
install until the cutover. It has Home and About so far. Linux builds need `libfontconfig-dev` as well.

```sh
cargo run -p td-app                                    # with your real Devices
cargo run -p td-app --features mock -- --mock-devices 2 # with mock Devices
cargo test -p td-app                                   # includes headless UI tests
```

`TOUCHDECK_APP_DATA_DIR` moves the app data folder, as it does for the Companion; the window
keeps its place and the close-notice flag in `window.json` there. `SLINT_BACKEND=winit-software`
selects Slint's software renderer.

The crate's own source is Apache-2.0 like the rest of the repository, but it links Slint under
GPL-3.0, so the program it builds is GPL-3.0-only. Its About page lists the licences of the
components the program uses directly; `cargo test -p td-app` checks that list against the Cargo
manifests.

## Develop and check changes

Run these commands from `app/`:

```sh
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Pull request CI runs the same three on Windows and Ubuntu. To run the Linux checks on the committed `HEAD`
in Docker (needs Docker; the first run takes a few minutes):

```sh
./scripts/linux-check.sh
```

To run the Companion with mock Devices and its test routes, build it with the `test-hooks` feature. Never
enable that feature for a distributed build:

```sh
cargo run -p td-companion --features test-hooks -- --mock-devices 1
```

### Windows Companion shell

The Companion sets the fixed process AUMID `io.github.duresa7.touchdeck` before
creating the tray. Its notification icon uses a permanent GUID and restores
itself after Explorer restarts. Left-click and Enter/Space open the app;
right-click and the context-menu key show the existing menu. The executable
embeds `crates/td-companion/icon.ico` as resource 1 through `app.rc`,
alongside the Common Controls v6 manifest, and loads it with `LoadIconMetric`.
Windows builds need the SDK resource compiler (included in the documented
MSVC setup); Linux cross-checks use MinGW's `x86_64-w64-mingw32-windres`.

`td_companion::tray::open_app(packaged)` is the single Windows app-opening
hook, shared by tray activation and Open. It still looks for the 1.x window program
next to the Companion, allowing `TOUCHDECK_WINDOW_BIN` only in development, and
`main` no longer builds that program. At Slint cutover, replace its body with a
dispatch to the native UI event loop.

Start at sign-in defaults to on at the first packaged start. Its reported state
requires both the per-user Run registration and Explorer's matching
`StartupApproved\Run` approval. That binary format is undocumented: the parser
recognizes enabled DWORD states 2/6 in twelve-byte records, treats missing
approval as allowed, and keeps disabled (3/7), malformed or unknown records off.
Initialization preserves Explorer's decision; only explicitly turning the app's
setting on clears that decision. Linux startup behavior is unchanged.

## Repository layout

| Path | Responsibility |
| --- | --- |
| `crates/td-core` | Frame codec, link messages, layout validation, settings, and source-config path/storage helpers. |
| `crates/td-link` | Serial and mock transports, transfers, firmware update and ROM recovery, and the link CLI. |
| `crates/td-host` | Host input, actions, approvals, icon processing, and Sources. |
| `crates/td-companion` | Background Companion and local API. |
| `crates/td-updater` | Finds, downloads and verifies a 2.x update. |
| `crates/td-app` | The 2.0 program: the Companion and the Slint window. |
| `installer` | The 2.0 MSI's WiX source. |
| `icons` | Built-in Tile icons and the app icon set. |
| `scripts` | Release, data and setup scripts, and their tests. |

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
