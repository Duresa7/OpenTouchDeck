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

The program is `td-app`, described under [Native app](#native-app-20-in-development). `td-companion` is
the library it runs in-process: the Devices, Integrations, actions and the tray. Many of its integration
tests still drive it through the 1.x local JSON API, which nothing else serves
([retired contract](../docs/companion-api.md)).

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
cargo build -p td-app
```

The program is `target/debug/td-app.exe` on Windows or `target/debug/td-app` on Linux. The
[packaging guide](../docs/packaging.md) describes the 2.0 MSI and its release pipeline.

## Native app (2.0, in development)

`crates/td-app` is the Open TouchDeck 2.0 program from
[ADR 0011](../docs/adr/0011-native-windows-app-in-rust-and-slint.md): the Companion and a
Slint window in one process. It is not released yet; the 1.x app on `release/1.x` is what people
install until the cutover. It has Home, Setup, Settings and About so far. Linux builds need `libfontconfig-dev` as well.

```sh
cargo run -p td-app                                    # with your real Devices
cargo run -p td-app --features mock -- --mock-devices 2 # with mock Devices
cargo run -p td-app --features mock -- --mock fresh,device,radio,board
cargo test -p td-app                                   # includes headless UI tests
```

`--mock` takes a list of mock kinds: `device`, `radio` (the radio build, with two paired apps),
`v1` (Firmware too old to keep a name), `board` (a New
board) and `fresh` (a New board whose Firmware install starts at once, so setup opens). Mock
builds accept every Firmware install on their own. For screenshots, `TOUCHDECK_MOCK_PAGE` opens
`setup`, `settings` or `device:<id>` first, `TOUCHDECK_TEXT_SCALE` sets the text size, so the
desktop's own settings stay as they are, and `TOUCHDECK_SNAPSHOT_DIR` saves the window as drawn
to `<dir>/<name>` whenever `<dir>/request` holds a file name. Settings > Appearance chooses light
or dark.

Setup and Settings follow the Companion through typed sessions in `td-companion`
(`devices::SettingsSession`, `devices::PairingsSession`, `setup::SetupSession`,
`features::firmware::FirmwareTask`); the window only turns their views into properties. Skipped
setups are kept in `setup-skipped.json` in the app data folder, and the Appearance choice in
`window.json`.

`TOUCHDECK_APP_DATA_DIR` moves the app data folder, as it does for the Companion; the window
keeps its place and the close-notice flag in `window.json` there. `SLINT_BACKEND=winit-software`
selects Slint's software renderer.

On Windows the program shows a notification-area icon from
`td_companion::tray`, and closing the window keeps the app running there (a one-time notice
says so). Left-click, Enter or Space on the icon opens the window; its menu has Open, Start at
sign-in and Quit. The tray reaches the window through an open hook, an
`Arc<dyn Fn() + Send + Sync>` that `td-app` builds from `App::opener()`, so the Companion crate
never depends on the UI. Quit calls `Handle::request_shutdown()`, and the program then runs the
Companion's bounded shutdown and removes the icon. `td-app.exe` embeds
`icons/app/open-touchdeck.ico` (resource 1, the executable and window icon), `icons/app/tray.ico`
(resource 2, loaded with `LoadIconMetric` at the notification area's size), the Common Controls
v6 manifest and a version resource; release builds use the Windows subsystem, so no console
opens. Linux has no tray yet, so closing the window ends the program.

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

To run the app with mock Devices, build it with the `mock` feature, which turns on td-companion's
`test-hooks`. Never enable either for a distributed build:

```sh
cargo run -p td-app --features mock -- --mock-devices 2
```

### Windows tray

The program sets the fixed process AUMID `io.github.duresa7.touchdeck` before
creating the window or the tray. The tray's notification icon uses a permanent
GUID and restores itself after Explorer restarts. Left-click and Enter/Space open
the native window; right-click and the context-menu key show Open, Start at
sign-in and Quit. Windows builds need the SDK resource compiler (included
in the documented MSVC setup); Linux cross-checks use MinGW's
`x86_64-w64-mingw32-windres`.

Start at sign-in defaults to on at the first packaged start (`--packaged`, the
command line the Run entry registers). The tray reads it each time its menu
opens. Its reported state
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
| `crates/td-companion` | The Companion library td-app runs in-process, and the test-only local API. |
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
