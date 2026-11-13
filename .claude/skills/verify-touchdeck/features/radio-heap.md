# Radio build internal RAM floors

The radio build (Wi-Fi, lwIP, NimBLE) keeps two internal-RAM floors on the board: 24,576 bytes free in steady state and 16,384 bytes at the lowest point since startup, which the Wi-Fi station's start sets at boot (#313). `docs/firmware.md` ("Radio build board check") has the background.

## Sub-features

- `radio-steady-floor`: 30 s after the station starts, `radio: steady free_internal=N largest_internal=N floor=24576` as an info line.
- `radio-boot-floor`: right after it, `radio: boot min_free_internal=N floor=16384` as an info line. Below a floor the line is an error with `below floor`.
- `radio-state-floors`: `radio state` replies with `floor_ok` and `boot_floor_ok` true, and `min_free_internal` at or above 16384.

## Driving it with touchdeck.ps1

Preconditions:

- A radio build, built into a directory of its own (`$TD idf -B <abs>/build-radio -DTD_RADIO_BUILD=1 build`) and flashed with the same `-B`. Doctor compares against `firmware/build`, so it reports `build_matches: false` for a radio build; save `$TD serial cmd "version"` as the build's identity instead and check its `elf_sha256` against the build's `touchdeck.elf` app descriptor.
- BLE stays off (the stored setting; `radio state` reports `ble` `off`), and no network is set. Never set Wi-Fi credentials for this recipe.
- Set `TOUCHDECK_ROOT` to the worktree under test, and run `$TD` through PowerShell.

Recipe:

```powershell
& $TD serial capture --out <abs>/radio-boot --seconds 40 `
    --expect "touchdeck: TouchDeck firmware starting" `
    --expect "radio: steady free_internal=\d+ largest_internal=\d+ floor=24576" `
    --expect "radio: boot min_free_internal=\d+ floor=16384" `
    --send "heap" --send "radio state" --send-after 36
```

Pass: exit `0`, both floor lines are `I (...)` lines, and the `radio state` reply in `serial.log` has `"floor_ok":true` and `"boot_floor_ok":true`. Record `steady free_internal` and `boot min_free_internal` in the report.

Reference (2026-11-07, BLE off, not joined): steady 69,719, boot minimum 30,692. Before #313: 52,963 and 13,364 (45,835 and 7,812 before the link stream moved to PSRAM in #305).

## Notes

- The boot minimum is ESP-IDF's minimum free size, the same figure as `heap`'s `min_free_internal`. ESP-IDF gives its 32 KB reserved internal pool a minimum of 0, so the figure leaves that pool out; real headroom is about 32 KB larger.
- A perf build (`perf/tools.defaults`) resets the minimums at `app_main`, so its numbers are not comparable with this recipe's.
- When done, flash the normal Firmware back (`$TD idf build`, `$TD idf flash`) and check `$TD serial cmd "link hello"` or Doctor.
