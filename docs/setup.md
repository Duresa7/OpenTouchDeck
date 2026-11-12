# Desktop setup for Open TouchDeck over USB

Open TouchDeck connects through USB-Serial/JTAG. The running Companion sends the Device's keys to the focused window on this PC. Bluetooth is not supported. Wi-Fi and battery support are postponed.

## Running a distributed app versus building it

A distributed app does not need Node, pnpm, Rust, or a C++ compiler. The Windows installer carries the Electron window, the Companion and the Firmware bundle; the [packaging guide](packaging.md) describes it. Linux source builds need a graphical desktop, Electron runtime dependencies, USB serial access and permission to create a virtual keyboard through `/dev/uinput`.

Source builds use **Node.js 24**, matching [app CI](../.github/workflows/app.yml), and the **exact pnpm version in [app/package.json](../app/package.json)** (currently `12.12.1`). Use stable Rust, at least the workspace's declared minimum in [app/Cargo.toml](../app/Cargo.toml). Firmware build prerequisites are separate from desktop setup.

## Windows source-build setup

Use Windows with [PowerShell 7](https://learn.microsoft.com/en-us/powershell/scripting/install/installing-powershell-on-windows) and Microsoft's App Installer/winget available. From the checkout root:

```powershell
pwsh -NoProfile -File app/scripts/setup-windows.ps1
```

This command **installs missing development tools**. Review it before running it; installers may need administrator consent or a restart. The script does not elevate itself, change execution policy, or persist PATH directly. It sets the default rustup toolchain to `stable-msvc`, adds clippy/rustfmt, installs missing MSVC C++ Build Tools, and checks every native command's exit code.

If Node is absent, setup asks winget for available LTS versions and installs the newest **24.x** entry with an explicit version. If the source no longer offers 24.x, it stops with a manual Node 24 installation link. An existing Node from another major is rejected: select/install Node 24, rather than letting setup replace a version-manager selection. pnpm is installed or updated through npm to the manifest pin. On npm 12 or later, setup passes `--allow-scripts=pnpm` so pnpm's installer can replace its placeholder with the native binary; earlier npm versions omit that flag. Setup reads the selected npm shim before invoking pnpm and repairs a shim targeting the extensionless placeholder without executing it. Its final version must match exactly. Other shims that exit successfully but print no version fail with a repair message.

Setup appends installed Machine/User PATH entries and standard Cargo/npm/Node locations to **its own process PATH**, preserving existing entries and their priority. This makes fresh installs usable in that run. A child script cannot update the terminal that launched it: **open a new PowerShell after setup**, then verify:

```powershell
node --version
pnpm --version
rustc --version
```

If a tool is missing or the version is wrong, inspect resolution before retrying:

```powershell
Get-Command node,pnpm,npm,rustup -All
```

Select Node 24 in your version manager or remove the obsolete PATH/shim entry using that tool's normal management procedure. For pnpm, repair npm first if necessary, then run this command on npm 12 or later (use the manifest's version if it changes):

```powershell
npm install --global pnpm@12.12.1 --allow-scripts=pnpm --foreground-scripts
```

Check `npm --version`; on npm 11, omit `--allow-scripts=pnpm` because older releases do not support it. npm 12 [blocks unapproved install scripts](https://github.com/npm/cli/blob/v12.0.0/CHANGELOG.md), and [`--allow-scripts=pnpm`](https://docs.npmjs.com/cli/v12/using-npm/config#allow-scripts) approves pnpm's scripts for this install. When those scripts are blocked, Windows may show a **Pick an app** dialog and print no pnpm version because the npm shim targets an extensionless shell placeholder. If pnpm still prints nothing after repair, repair/remove the stale shim shown by `Get-Command pnpm -All`; a zero exit code alone is insufficient. Do not lower execution policy to make a broken PowerShell shim work; setup uses native/`.cmd` launchers.

Rerunning setup retains working Node/pnpm versions and skips their installers. Rustup's default/component operations are repeatable. The MSVC installation is skipped when detected and checked again after an installer succeeds. A nonzero installer exit, incomplete registration, restart requirement, or native command failure stops setup; resolve the reported problem and rerun. A successful version summary does not replace a full app build.

## Linux source-build dependencies

Ubuntu **24.04** is the Linux CI target. Its build packages are:

```sh
sudo apt-get update
sudo apt-get install -y build-essential libudev-dev libfontconfig-dev
```

This matches [the workflow](../.github/workflows/app.yml), with `libudev-dev` for serial discovery. Other distributions need their equivalent packages. Install Node 24, the pinned pnpm, and stable Rust separately using their upstream instructions.

From `app/`, the ordinary source build is:

```sh
pnpm install --frozen-lockfile
pnpm build:ui
cargo build -p td-companion
pnpm --filter @touchdeck/electron build
```

`libfontconfig-dev` is for the native app's Slint UI (`td-app`), which `cargo test --workspace` builds. Use `pnpm dev:electron` to run the Electron window with a mock Device. Use `pnpm dev:ui:demo` from `app/` (or `pnpm dev:demo` from `app/ui/`) for the React UI with demo data and no hardware. Use `pnpm lint`, `pnpm test`, `pnpm e2e:ui` and `cargo test --workspace` to check changes.

Do not enable `test-hooks` for a distributed app. To build the Windows installer, follow [Build the installer](packaging.md#build-the-installer).

## Linux runtime libraries and device access

Build headers and compilers are not runtime permissions. The Electron window uses its bundled Chromium, not WebKitGTK. Source builds need the shared libraries required by the installed Electron version and `libudev1` for serial discovery. A portable archive does not imply that all runtime libraries are bundled.

Before using a connected deck, inspect the actual nodes without opening them or generating input (replace `ttyACM0` with the deck's node):

```sh
id
ls -l /dev/ttyACM0 /dev/uinput
getfacl /dev/ttyACM0 /dev/uinput
udevadm info --attribute-walk --name=/dev/ttyACM0
test -r /dev/ttyACM0 && test -w /dev/ttyACM0
test -w /dev/uinput
```

`getfacl` is supplied by the distribution's ACL utilities. These checks inspect access; they do not prove a successful serial connection or virtual keyboard creation.

- **Serial:** the app needs read/write access to the deck's serial node. Its discovery filter is Espressif VID `303a`, PID `1001` ([code](../app/crates/td-link/src/lib.rs)). Close serial monitors and flash tools before connecting; another process may hold the port. Permission denial can also appear as a busy port. Debian's [`dialout` group](https://wiki.debian.org/SystemGroups) grants serial access, but covers other serial devices too. Prefer an administrator-managed device-specific ACL or a dedicated group rule matching this deck, including its actual USB serial attribute when available. Log out and back in after any group membership change.
- **Keyboard input:** the kernel must provide `uinput`, and the app's user must be allowed to write **`/dev/uinput`**, the exact path opened by [the Linux input backend](../app/crates/td-host/src/input_linux.rs). A system exposing only `/dev/input/uinput` needs an administrator-managed compatible device path. If the module is absent, an administrator must enable it through the distribution's module configuration. Serial access alone does not grant keyboard access. The [kernel's uinput documentation](https://docs.kernel.org/input/uinput.html) explains the interface.

For an administrator who chooses dedicated groups, the following are **rule templates**, not automatic setup. Create the groups, explicitly add only trusted users, replace the serial placeholder with the inspected value, and review existing rules first. Local rules belong in `/etc/udev/rules.d/` and need the distribution's normal rule reload/device-event handling:

```udev
SUBSYSTEM=="tty", ATTRS{idVendor}=="303a", ATTRS{idProduct}=="1001", ATTRS{serial}=="REPLACE_WITH_THIS_DECK_SERIAL", GROUP="touchdeck-serial", MODE="0660"
SUBSYSTEM=="misc", KERNEL=="uinput", GROUP="touchdeck-uinput", MODE="0660"
```

See the [udev rule and permission reference](https://www.freedesktop.org/software/systemd/man/latest/udev.html). Granting uinput access allows processes under that account to inject system input, regardless of which physical device triggered it. Restrict it to a trusted account/session. Do not make device nodes world-writable, grant access to all input devices, disable desktop security controls, or run the desktop app as root. If local policy disallows virtual keyboard creation, leave that permission unavailable; mirrored typing will not work.

## Integration credential storage

The CLIProxyAPI management key is stored through the OS credential store, not in the ordinary Integration configuration ([implementation](../app/crates/td-host/src/integrations/secrets.rs)). The locked `keyring` 4.2 backend uses **Windows Credential Manager** on Windows, **Keychain Services** on macOS, and **Secret Service** on Linux ([upstream platform mapping](https://docs.rs/keyring/4.2.0/keyring/v1/index.html)). This is a credential-store mapping, not a claim that every app feature supports macOS.

On Linux, run the app in the intended user's desktop login session with a session D-Bus and an available Secret Service provider, such as GNOME Keyring or a compatible configured wallet. Its default collection must be usable/unlocked when saving or reading the key; an unlock prompt may be required. See the [Secret Service specification](https://specifications.freedesktop.org/secret-service/latest/description.html). Minimal desktops, headless sessions, containers, and CI may lack that service. Install/configure the provider through the distribution's documented desktop setup if you want this Integration. Do not work around credential-store errors by saving keys in scripts, logs, or world-readable files.

## Verification boundary

Linux CI builds the app and runs its unit and Rust tests on mock Devices; the UI e2e specs run on Windows only. Mock Devices and fake input do not exercise hardware, and mock Integrations use an in-memory key store. Those checks establish neither real serial permissions, real `/dev/uinput` operation, nor a usable desktop credential store. The setup changes were verified with isolated Windows process fixtures; no real installers, Linux runtime session, device permission changes, or real key injection were exercised in that validation.

The focused setup tests require Windows and PowerShell 7, with no Pester or installed Node/Rust/pnpm dependency:

```powershell
pwsh -NoProfile -File app/scripts/tests/setup-windows.test.ps1
```

Each case starts a fresh process with temporary tool stubs and a guarded tool resolver. Installer calls, tool versions, PATH additions, and native failures are simulated; the suite does not install tools or modify host settings.

## Git hooks

Enable the committed hooks once per clone (shared by its worktrees), from the repository root:

```sh
git config core.hooksPath .githooks
```

Python 3 is required (`python3`, `python`, or `py -3`). Pre-commit scans added
staged lines for private data and, when a Rust file is staged, runs CI's
`cargo fmt --check` from `app/`. Missing Cargo skips only the format check.
Pre-push scans added lines and messages of outgoing commits; new branches exclude
commits already reachable from remote-tracking refs. Keep those refs current.
Both hooks block on findings: remove the data and retry. They run no tests or clippy.

Existing executable hooks in the common Git directory's `hooks/` continue to run
after the shared checks pass, with their arguments and pre-push input preserved.
The pass-through post-checkout hook also preserves local worktree setup.

For local-only exact values, edit `info/private-patterns` under the directory
reported by `git rev-parse --git-common-dir`. Every worktree shares this file;
never commit it. Blank lines and lines starting with `#` are ignored. Plain lines
are case-insensitive literals; `re:` introduces a Python regex (case-sensitive
unless it includes a flag). For example:

```text
# Placeholder examples; replace locally
<private-email>
<private-lan-address>
<private-hostname>
re:(?i)<private-user-name>
```

Run `python3 .githooks/scanner.py --all-files` to scan every tracked file at HEAD
with generic rules only. Exact-line SHA-256 entries in `.githooks/allowlist.json`
document existing false positives without publishing matched contact details;
each entry is restricted to a file and rule and requires an explanation. Local
patterns cannot be allowlisted. Private IP ranges are not flagged generically.
Run the scanner tests with `python3 -B -m unittest discover -s .githooks -v`.
