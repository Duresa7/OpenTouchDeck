# Storage and settings

The deck keeps its files on one LittleFS volume at `/storage`, on the `storage` partition. A write goes to `<path>.tmp`, is flushed, and is renamed over the target, so a reader sees the whole old file or the whole new one. The settings file `/storage/settings.json` holds the idle timeout and the orientation, and the deck reads it before the display starts. The `store` and `settings` console commands read and write all of it.

## Sub-features

- `format`: the first boot after the partition was erased logs `store: formatted`, and every boot logs `store: mounted, <used>/<total> bytes`.
- `settings`: `settings.json` is read at boot, logged as `settings: idle_timeout_s=<n> orientation=<o>`, and the timeout reaches the power gate. A bad field takes its default.
- `put`: `store put` takes a file as `@tds` base64 lines and swaps it in atomically after its last line.
- `get`: `store get` streams a file as `@tds` lines between a `start` and an `end` reply with a CRC-32.
- `inspect`: `store stat`, `store ls`, and `store rm` report and remove files.
- `survive`: a flash, a reboot, and a reset in the middle of a put leave the volume whole.

## How to get to it (user POV)

- The app, from M2-05 on, sends layouts and icons into this volume. Until then the console is the only way in.
- `settings set idle_timeout_s <n>` and `settings set orientation landscape|portrait` change the settings. The timeout applies at once. The orientation is stored and reported, and the display applies it from M2-07 on.
- Flashing the firmware leaves the volume alone. Only `esptool erase-region 0x840000 0x7C0000` empties it.

## Driving it with touchdeck.ps1

Preconditions: the board lock is held, the build under test is flashed, and Doctor exits `0` with `build_matches` true. `$TDH` below is the same `touchdeck.ps1` once the root has installed the `store` subcommand.

- First boot after an erase. `$TD esptool --after no-reset erase-region 0x840000 0x7C0000` leaves the chip in the bootloader. Then `$TD serial capture --seconds 8 --out <dir> --expect 'store: formatted' --expect 'store: mounted, \d+/\d+ bytes' --expect 'settings: idle_timeout_s=600 orientation=landscape' --expect 'touchdeck: alive 0'` does the reset itself, so it records that first boot. A flash and a plain Doctor reset the board first and use up the first boot. An erased volume also logs two `esp_littlefs` error lines, `Corrupted dir pair` and `mount failed`, before the format, which is the library saying it found nothing to mount.
- Volume report. `$TD serial cmd "store stat"` replies `{"cmd":"store","ok":true,"op":"stat","path":"/storage","dir":true,"files":N,"dirs":N,"formatted":false,"total":8126464,"used":8192}`. A fresh volume uses 8192 bytes, one metadata pair. `store stat <dir>` counts that directory's files and dirs below it, and `store stat <file>` gives `size`.
- Listing. `$TD serial cmd "store ls /storage"` replies one line with `count`, `truncated`, and `names`, sorted, with a `/` after each directory. It names at most 64 entries and sets `truncated` when there are more.
- Persist a setting. `$TD serial cmd "settings set idle_timeout_s 120"`, then `$TD serial capture --no-reset --seconds 8 --out <dir> --send reboot --send-after 0.5 --expect 'settings: idle_timeout_s=120 orientation=landscape' --expect 'touchdeck: alive 0'`, then `$TD serial cmd "power state"` reports `idle_timeout_s` 120.
- Put and get. `$TDH serial store put /storage/test.bin --from <file>` and `$TDH serial store get /storage/test.bin --out <file>` print one JSON object with `ok`, `bytes`, `lines`, `crc32`, `seconds`, and `bytes_per_s`, and exit `0` when every check passed. Put checks that the deck's `done` reply carries the CRC-32 and size it sent. Get checks the byte count, the line count, and the CRC-32 against the `end` reply, and writes `--out` only when all of them pass. Exit `1` for a failed check or a firmware error, `2` for a missing port or no reply. A 60000-byte file went in at about 120 KiB/s and out at about 190 KiB/s.
- A reset in the middle of a put. A burst of `capture --send` lines overruns the console's 2047-byte input queue, so send the lines in windows. `request_store_put` in `touchdeck_serial.py` does, and a script that imports it can stop after any line and send `reboot`. The deck holds a put in memory and writes nothing until the last line, so the next boot finds the old file whole and no `<path>.tmp`.
- A stream inside a capture. `$TD serial capture --no-reset --seconds 11 --out <dir> --send "store get /storage/test.bin" --send-after 3`, then `$TDH serial store decode <dir>\serial.log --out <file>` rebuilds the file from the last complete stream and checks its CRC-32. The `alive` lines in the same log show whether the heartbeat held.
- Bad settings. `store put /storage/settings.json --from <file>` with `{"orientation":"sideways","idle_timeout_s":0}`, then `reboot`, boots to `settings: idle_timeout_s=600 orientation=landscape`. `settings get` before the reboot reads the file as the next boot would, with `stored` true.
- Clean up. `store rm <path>` removes a file or an empty directory, so a lane removes what it made.

The wire form, for a script that does not use the harness. `store put <path> <bytes> <crc32>` answers `{"op":"put","phase":"ready","line_bytes":180,"ack_lines":6}`. Then the host writes `@tds <base64>` lines of 180 raw bytes, the last one shorter, and waits for `{"op":"put","phase":"ack","got":N}` after every 6 lines. After the last line the deck answers `{"op":"put","phase":"done","crc32":N,"bytes":N}`, or an `ok` false reply with `crc mismatch`, `bad line`, `bad line length`, or `write failed`. `crc32` is `zlib.crc32` as a decimal. `store get <path>` answers `{"op":"get","phase":"start","bytes":N,"lines":N}`, then `@tds <base64>` lines of up to 180 bytes, then `{"op":"get","phase":"end","bytes":N,"crc32":N}`.

## Gotchas

- Paths are `/storage/` followed by segments of letters, digits, `.`, `_`, and `-`, at most 32 characters each and 95 in all. A path with a space, a `..`, an empty segment, or a trailing `/` gets `invalid path`. A path ending in `.tmp` is refused for a put.
- A put takes 1 to 65536 bytes. The `serial store put` command refuses other sizes before it touches the port.
- The console's typed echo repeats every `@tds` line the host sends, so a reader sees them too. The harness skips them. `capture --send` is for a single `store` line, not for a transfer.
- `store get` and `store put` block the console while they run. Run nothing else on the port meanwhile, as with `screenshot`.
- Stale `*.tmp` files, left by a write that lost power before its rename, are removed at the next mount, and the boot logs a warning when it removes one.
- A flash leaves the volume alone, but the unit test app and the deck share the partition and the unit tests never mount it. `esptool erase-region` is the only thing that empties it.
- Lanes share one volume, so a lane sets up the files and settings it needs and removes them after. Counts such as `files` hold for a directory the lane owns, not for the whole volume.
- The free heap at `alive 0` moves in 64 KB steps. The PSRAM mapping rounds the code and the read-only data up to whole 64 KB pages, so a change that pushes the code image over a page boundary costs 64 KB at once.
