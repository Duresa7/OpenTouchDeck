# Firmware update and rollback

From M2-09 on, the app updates the deck's firmware over the link, and the bootloader rolls back to the previous image when the new one does not come up. The flash holds two 4 MB slots, `ota_0` and `ota_1`. An update writes the slot the deck is not running, checks the SHA-256 of the bytes it wrote, validates the image, makes the new slot the boot slot, and restarts 250 ms after its reply. The new image boots `PENDING_VERIFY` and confirms itself after its first frame, a layout load, and 10 seconds of uptime. A reset before that makes the bootloader mark the slot `ABORTED` and boot the old one.

## Sub-features

- `update-begin` is `update.begin {bytes, sha256}`. It refuses a size of 0 or over the slot (`size`), a hash that is not 64 hex digits (`sha256-format`), a second update while one runs (`busy`), and any update while the running image waits to confirm (`unconfirmed`). It replies with `xfer` and `slot`.
- `update-transfer` is the M2-05 chunk transfer, window of four, with the image written to the slot as it arrives. A transfer with no chunk for 2 seconds is dropped and the state returns to `IDLE`.
- `update-end` is `update.end {xfer}`. It replies `reason` `incomplete`, `sha256`, `flash`, or `project` and leaves the boot slot alone, or `slot` and a restart.
- `update-confirm` logs `update: confirmed <slot>` once the image is up. An image that was never pending, as after a cable flash, reports `confirmed` true from the start.
- `update-rollback` is the bootloader's, not the app's. A reset while the slot is `PENDING_VERIFY` marks it `ABORTED` and boots the other slot.
- `update-state` is the console command `update state`: `running`, `boot`, `ota_0`, `ota_1` (`NEW`, `PENDING_VERIFY`, `VALID`, `INVALID`, `ABORTED`, `UNDEFINED`, or `NONE` for a slot with no record), `confirmed`, `state` (`IDLE`, `RECEIVING`, `VERIFYING`, `REBOOTING`, `FAILED`), `bytes`, `written`, and `reason` while `FAILED`.
- `update-test-hooks` are `update confirm`, which confirms at once, and `update invalidate`, which marks the running image invalid and reboots into the other slot when that slot holds a valid image.
- `boot-crash-build` is `$TD idf build-crashing`: the firmware built with `-DTD_TEST_BOOT_CRASH=1` into `firmware/build-crash/`, which aborts 2 seconds after `app_main`.

## How to get to it (user POV)

- The app sends a `.tdfw` bundle's `touchdeck.bin` through `update.begin`, the chunks, and `update.end`. The deck goes dark for a moment, restarts, and shows its pages again on the other slot.
- A bad image leaves the deck on the old build. It restarts into the old slot by itself within seconds.

## Driving it with touchdeck.ps1

Preconditions:

- Hold the board lock, use PowerShell, set `TOUCHDECK_ROOT` to the tested worktree, and pass absolute paths.
- Flash the build under test first, `$TD idf flash`, and run Doctor. The cable flash writes `ota_0` and resets the boot slot to it, so the first update goes to `ota_1`. **The bootloader must come from the same build.** The update never rewrites the bootloader, so a bootloader without rollback enabled makes the check below prove nothing. Doctor's `boot_slot` and `otadata` show where the deck stands.
- Never take USB away from USB-Serial/JTAG. Nobody can press BOOT. The crashing build keeps USB-Serial/JTAG, and a cable flash recovers a deck whatever the slots hold.

Commands:

- Update. `$TD link update <abs>\firmware\build\touchdeck.bin --wait --out <abs>\update.json --log <abs>\update.log`. It reads `hello` and `update state`, waits up to 30 seconds for `confirmed`, sends `update.begin`, streams the chunks, sends `update.end`, waits for the port to come back, and exits `0` only when `hello` reports the image's `elf_sha256` on the slot `update.begin` named and `update state` reports that slot `VALID` and `confirmed` true within 20 seconds of the first `hello`. Without `--wait` it stops after `update.end`.
- Rollback. Run `$TD idf build-crashing` first. Then `$TD link update <abs>\firmware\build-crash\touchdeck.bin --wait --expect rollback --out <abs>\rollback.json --log <abs>\rollback.log`. It exits `0` only when `hello` reports the old image's `elf_sha256` on the old slot again and `update state` reports the slot it wrote `ABORTED`. `rollback.log` holds the console lines of the whole session, so it shows the abort backtrace and the second boot where the port stayed up.
- State. `$TD serial cmd "update state"`.
- Doctor. `$TD serial doctor` adds `boot_slot`, the slot the bootloader runs, and `otadata`, both entries as `{sector, ota_seq, state, crc_ok, valid}`. `flashed_app` is the descriptor of the boot slot, so after an update to `ota_1`, `build_matches` compares that slot. The boot slot is the valid entry with the higher `ota_seq`, slot `(ota_seq - 1) % 2`, where a valid entry has a good CRC and a state other than `INVALID` or `ABORTED`. With none, `ota_0` runs.
- Return to `ota_0`. `$TD idf flash`. It writes `ota_data_initial.bin`, so the deck boots `ota_0` with no record for either slot, and `update state` reports them `NONE`.
- Newer bundle (test only). `$TD idf build-newer [--out-dir <abs dir>] [--version <v>]` builds the same source into `firmware/build-newer/` with `PROJECT_VER` one patch above `git describe`, packs and checks it, and prints the bundle's JSON. The image differs from `build/`, so a Companion built with it (`verify-touchdeck-app`: `$TA build --firmware-bundle`) offers and sends a real update. Procedure and the way back to main are in that skill's `features/board-pass-tools.md`.
- Bundle. `python firmware/tools/make_bundle.py` packs `bootloader.bin`, `partition-table.bin`, `touchdeck.bin`, and `manifest.json` into `touchdeck-<version>.tdfw`. `--check <file>` reads one back.

## Gotchas

- The old image answers for a moment after `update.end`, until the restart 250 ms later. The command waits 1.5 seconds and reads `elf_sha256`, not the port, so it never mistakes the old image for the new one.
- A deck that was just flashed and has not confirmed refuses `update.begin` with `unconfirmed`. The command polls `update state` for up to 30 seconds first.
- The crashing image dies 2 seconds after start, so a `hello` may land on it for an instant. Judge the end state, not the middle.
- A rollback takes about 5 to 10 seconds from `update.end`: restart, the crashing boot, the abort, and the old image's boot.
- `update state` reports the slot the bootloader marked `ABORTED` only after the second boot. Before it, that slot reads `PENDING_VERIFY`.
- Every restart ends the console session. Reopen the port in a new `$TD` call, or let `link update --wait` do it.
- Only the board shows that a real update survives the reset. A serial log of the new image booting is the proof, so keep `--log` and the JSON, and report anything about the panel itself as pending operator.
