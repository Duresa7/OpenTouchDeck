# Bridge upgrade check

`.github/workflows/bridge-upgrade.yml` proves that Open TouchDeck 1.x moves to 2.0 through the bridge and keeps the
user's data (issue #357). The bridge itself lives on `release/1.x`: `app/electron/src/updates/bridge.ts` finds and
verifies the 2.0 MSI, and `app/electron/installer/bridge.ps1` installs it and removes 1.x. [docs/packaging.md](../../../docs/packaging.md#the-move-from-1x)
describes both.

## What runs

Three jobs on GitHub-hosted `windows-latest` runners:

- **bridge** builds the 1.x bridge release from `release/1.x` (or the branch in `BRIDGE_REF`) with the 1.x recipe in
  test-signing mode. It reuses the published v1.2.0 Firmware bundle instead of building one with ESP-IDF.
- **msi** builds `td-app` and the 2.0 MSI from this commit, an MSI that always fails (`failing.wxs`), and
  `bridge_data`, the report tool (`app/crates/td-companion/examples/bridge_data.rs`).
- **upgrade** runs `upgrade.ps1`.

`upgrade.ps1`:

1. Installs the published 1.2.0 and seeds data in 1.x's own formats: a Device, its Layout and a Pairing as files
   (`seed/`) with the Pairing key in Credential Manager, then the CLIProxyAPI Integration and its secret through
   1.x's API (`onex_api.mjs`), so 1.x writes them itself. It checks that 1.x lists all of it.
2. Updates 1.2.0 to the bridge build through the real 1.x updater (`lifecycle_update.mjs` on `release/1.x`), the
   NSIS installer with `/S --updated --force-run`, and checks the data again.
3. Signs both MSIs with a throwaway updater key made on the runner and deleted at once. The 1.x updater is run
   against a loopback stand-in for github.com and its release list (`bridge_update.mjs` on `release/1.x`).
4. **Tampered MSI.** The updater offers 2.0, then refuses the MSI. Nothing changes.
5. **Failing MSI.** The bridge runs it and Windows Installer fails with 1603 and rolls back. The 1.x window is
   left open, so `bridge.ps1` has to end it. 1.x is back as it was: folder, uninstaller, uninstall entry, Start menu
   shortcut, start at sign-in, data, and 1.x starts again and reads the bridge result.
6. **The 2.0 MSI.** The window is closed first, as the real updater quits it. The bridge installs 2.0 and starts it.
   The check expects Windows Installer to list 2.0 for the current user only. The install folder must hold only
   `td-app.exe`. Nothing of 1.x may be left: no uninstall entry, install record, Start menu or desktop shortcut,
   window profile, staged MSI or process. Start at sign-in must now run 2.0, and 2.0's Home must list the seeded
   Device. Then `bridge_data report` reads the data through 2.0's code: the Device, its Layout, the Pairing and its
   key, the Integration and its secret. Every user data file and Credential Manager entry from before is still
   there.

It uploads `summary.json`, the updater results, 1.x's and 2.0's views of the data and the bridge and MSI logs,
with the runner's user name, PC name and profile folder removed.

## What it can't prove

- **Windows 11 itself.** `windows-latest` is Windows Server 2025, build 26100, the Windows 11 24H2 code. The
  Windows 10 refusal is covered by the bridge's unit tests and by the MSI check on Windows Server 2022.
- **The updater key and 1.2.0's own updater code.** The real key can't sign on a test runner, so the check signs with
  a throwaway key and drives the updater module from the bridge branch. The update from 1.2.0 runs that module, not
  the code inside the installed 1.2.0, though it uses the same feed, verification and installer flags.
- **The 1.x window's click path and its message box.** The updater runs in Node, as the 1.x installer lifecycle
  check runs it. The check sees that 1.x read and removed the bridge result, not the message it showed.
- **A real Device or a cancelled MSI.** The seeded Device is never connected. Cancelling (exit 1602) takes the same
  rollback path as the failing MSI.

## Run it by hand

```sh
gh workflow run bridge-upgrade.yml --ref <branch> -f bridge-ref=release/1.x
```

`upgrade.ps1` refuses to run outside GitHub Actions: it installs and removes software and writes the real per-user
registry and Credential Manager.
