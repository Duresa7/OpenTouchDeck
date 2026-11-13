# Board pass tools

Three tools for board-pass steps that need more than the plain Companion: the Companion's own log, a Firmware bundle newer than the Device, and a console flood. The last section lists the open steps of a board pass checklist that each one unblocks. Everything here is test-only: nothing adds a Companion code path, and a Companion built without `test-hooks` still passes `python app/scripts/package_windows.py verify-companion <exe>`.

## Companion log

`$TA start` sends the Companion's stdout and stderr to `<data>/companion.log`, or to `--log <absolute file>`, and returns the path as `log`. The file is appended to, and each start first writes `--- tdapp start <UTC time> <command line>`, so several starts on one data folder stay apart. The file stays after `$TA stop`.

The log holds what the Companion prints on its own: the fatal error that ends it, port and hello problems such as `<port> gave an invalid v2 hello (...); using protocol 1.`, Device id fallbacks, failed saves and failed Integration polls. It does not record link frames, `v2.input` or console lines. Capture those with `$TA events --seconds <n>` (console lines are `device.console` events) or `$TA devcmd`. Attach the log to any step whose Device link fails, such as a #213 repeat.

## A Firmware bundle newer than the Device

The Companion offers the bundle it was built with. `build.rs` reads the `.tdfw` named by `TOUCHDECK_FIRMWARE_BUNDLE` and fails the build unless `td_link::bundle::Bundle::parse` accepts it (zip with the manifest and three files, every file's size and SHA-256, safe offsets, the TouchDeck application descriptor and its `elf_sha256`, the partition map). `GET /firmware/bundled` then reports its `version` and `build`. An update with `{"bundle":"bundled"}` ends at `checking` without sending anything when the Device's `elf_sha256` equals the bundle's, so a bundle that only re-stamps the version in `manifest.json` (`make_bundle.py --version`) proves nothing. The image itself has to differ.

The UI offers the bundle when its major.minor.patch is above the Device's (`newerFirmware` in `app/ui/src/lib/device/needs-firmware.ts`), and the Companion asks for native consent only when the bundle's semantic version is lower than the Device's. Mock Companions always bundle the td-link test fixture instead, so only a real Companion offers a staged bundle.

Procedure, with the Device on main's build from the same checkout:

1. `$TD idf build-newer [--out-dir <abs dir>] [--version <v>]` (the `verify-touchdeck` skill, PowerShell, `TOUCHDECK_ROOT` set). It builds the same source into `firmware/build-newer/` with `PROJECT_VER` one patch above `git describe` (`v0.2.0-508-g3eab778` gives `v0.2.1-newer-508-g3eab778`), packs it with `firmware/tools/make_bundle.py`, checks it, and prints `{ok, bundle, bytes, version, elf_sha256}`. `firmware/build/` stays main's build. `--version` takes 1 to 31 characters of `A-Z a-z 0-9 . _ + -`.
2. `$TA build --firmware-bundle <abs .tdfw>`. The result's `firmware_bundle` names it. A bad bundle fails the build with `TOUCHDECK_FIRMWARE_BUNDLE is not a valid Firmware bundle: <reason>`. Without the option the build uses whatever `TOUCHDECK_FIRMWARE_BUNDLE` the caller's environment holds.
3. With the hardware lock: `$TA start [--device-console]`, `$TA api GET /firmware/bundled` (expect the new version), then press Update in the Device panel or `$TA api POST /devices/<id>/firmware/update --body '{"bundle":"bundled"}'` and poll `$TA api GET /jobs/<job id>` through `sending`, `rebooting` and `confirming` to `succeeded`. Doctor afterwards shows the new `elf_sha256` on the other OTA slot.
4. Rollback (#77 step 2): `$TD idf build-crashing`, then `python firmware/tools/make_bundle.py --build-dir firmware/build-crash --out-dir <abs dir> --version <v>` with a version above the Device's, and `$TA build --firmware-bundle` with that file. The crashing image differs from the Device's, so it is sent, aborts two seconds after boot, and the bootloader puts the old slot back. Expect the Job `failed` with `firmware.rolled_back`. Here `--version` only re-stamps the manifest, which is fine because the image already differs.

Back to main afterwards:

- Device: `$TA stop`, then `$TD idf flash` from main's `firmware/build/`. The cable flash writes `ota_data_initial.bin`, so the Device boots `ota_0` with main's image. `$TD serial doctor` must show `build_matches` true. Updating back through the Companion would be a downgrade that waits for native consent, so use the cable.
- Companion: `$TA build` again, with `--firmware-bundle` pointing at main's bundle or with none, and check `$TA api GET /firmware/bundled`.
- `firmware/build-newer/` and `firmware/build-crash/` are git-ignored and can stay.

## Console flood

No Firmware command exists to flood the console. `screenshot` comes closest: after its `@td` start reply it streams the screen as `@tdb` lines of 768 bytes in base64, 1600 lines and about 1.65 MB at scale 1, 400 at scale 2, 100 at scale 4, then a trailer. The Companion publishes every line as a `device.console` event and its console queue holds 192 lines, so `$TA devcmd "screenshot 1"` (PR #212's route, Companion started with `--device-console`) saturates it for about two seconds. `devcmd` returns at the start reply while the stream goes on. A second `screenshot` sent meanwhile waits on the Device's console and starts when the first ends, so a few sent back to back give a sustained burst.

Limits:

- The Device's console runs one command at a time, so `keys` and `touch` lines sent during the stream wait until it ends. Injected input can come before or after a burst, not during it. Input during a burst needs a person's finger on the glass, or the test-only command below.
- While a Firmware update Job holds the Device, the Companion refuses `test.console` (the Device reads as updating), so nothing can start a flood during update confirmation.
- `perf`, `keys log on` and `touch log on` print one line per event, which is far too little to saturate anything.

Proposed test-only Firmware command for the cases screenshot can't cover (not built): a `-DTD_TEST_CONSOLE_FLOOD=1` build option, like `TD_TEST_BOOT_CRASH`, that adds `flood <lines> <bytes>`. It replies at once and prints the lines from a low-priority task, so the console stays free for `keys` and `touch` during the burst. With the option set to `boot` the image also starts one flood when it boots, so an image staged with `build-newer` floods during its own update confirmation.

## Open board-pass steps each tool unblocks

From `.verify/hardware-pass/2fab801/checklist.json` (board pass run 2):

| Tool | Steps |
| --- | --- |
| Companion log | #96 step 2 (#213 repeat: the Companion's side of the dropped link); any step whose link fails, as attached evidence. It doesn't replace `$TA events` for #97/#98 Step H steps 13 and 14, which read `v2.input` and `press` frames. |
| Newer bundle | #77 step 1 (update to the bundled build), #77 step 2 (crash bundle, rollback), #84 step 3 (Device panel Update with a newer bundle), #108 event preservation step 3 (update confirmation with normal console output; the flooding half stays open). |
| Console flood (`screenshot` through `devcmd`, needs PR #212) | #108 event preservation step 1 (console-only saturation keeps the link open; held keys around a burst, but not during one) and step 2 (Preset tile presses before and after bursts). Input during a burst and the flooding half of #108 event preservation step 3 need a person or the proposed `flood` command. |
