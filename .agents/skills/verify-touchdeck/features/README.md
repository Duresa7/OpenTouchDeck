# TouchDeck verification map

This directory is the maintained source for verifying the user-visible behavior of the TouchDeck firmware. Read the index before driving the board, then use the matching feature file as the recipe.

## Baseline preconditions

- The Waveshare ESP32-S3-Touch-LCD-5B is on USB and shows up as one Espressif USB-Serial/JTAG port (`$TD serial ports`, `303a:1001`; set `TOUCHDECK_PORT` to pin it).
- No other program holds the port. Close the VS Code ESP-IDF monitor if Doctor reports `port_free: false`.
- The build under test is flashed: `$TD idf build`, `$TD idf flash`, then `$TD serial doctor` exits `0` with `build_matches: true`.
- Build one tree at a time. Two ESP-IDF builds at once crashed gcc with an internal compiler error in `lv_obj_style.c` during M1-12, and each tree then built alone with 0 warnings.
- `$TD` is `pwsh -NoProfile -File .claude/skills/verify-touchdeck/scripts/touchdeck.ps1`, run from the repo root.
- Never flash over firmware whose `project_name` is not `touchdeck` without the user's go-ahead.

## Driving conventions

- Boot recipes drive the board through `$TD serial capture`, which resets the board like a power-on and records the serial log. From M1-03 on, console recipes use `$TD serial cmd`, which does not reset it. From M1-04 on, display recipes use `$TD serial screenshot`, which does not reset it either. From M1-05 on, touch recipes inject through `$TD serial cmd "touch ..."`, or through `$TD serial capture --send` when the log lines that the touch causes are the evidence.
- Start each capture from a reset unless the recipe says `--no-reset`. USB-Serial/JTAG drops output printed before the port opens.
- From M1-12 on, run `$TD serial cmd "power state"` before any touch recipe. A board left idle longer than the idle timeout, 600 seconds by default, has its screen off, and the first injected touch only wakes it, so a tap, swipe, or hold that lands then fires no tile and leaves no record entry. If `state` is `OFF`, wake the screen with one `touch tap 266 128`, the gap between F1 and F2 on page 0, or run `$TD serial cmd "power idle-timeout 86400"` for a long run and restore 600 afterward. At older links `power` is an unknown command, so the check costs one exit `1` and nothing else. [Idle](./idle.md) has the timing rules.
- Treat regexes as literal recipe text. Escape `(` `)` as written.
- One run per evidence directory: `.verify/<YYYYMMDD-HHMMSS>-<feature-id>/`.

## Proof and skip reporting

- A proof is `doctor.json` (`build_matches: true`) plus `serial.log` and `summary.json` with `"ok": true`, all in the run's evidence directory.
- Repeating behaviors need several occurrences with their timing checked from `summary.json`, not a single line.
- Report a failed expectation with its regex and the last lines of `serial.log`. Never loosen a regex to make a run pass.
- From M1-04 on, a screenshot shows LVGL's render of the active screen, which is what the firmware drew. Drift, flicker, tearing, and backlight state are visible only to the operator looking at the real panel. Report those as not verified, never as verified from logs or a screenshot.
- From M1-05 on, `touch tap` and `touch swipe` inject touches at LVGL's input device. That proves the UI logic and LVGL's input handling, and it does not exercise the GT911 or the glass. Accuracy, orientation, mirroring, and feel need a finger on the panel, so report them as pending operator, never as verified from an injected touch, logs, or a screenshot.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the user-visible behavior. It then uses exactly four H2 sections in this order.

1. `Sub-features` lists short IDs with one line for each behavior.
2. `How to get to it (user POV)` lists every user entry point.
3. `Driving it with touchdeck.ps1` starts with `Preconditions:` and uses labeled bullets that pair each user action with an exact command and observable result.
4. `Gotchas` lists traps that can waste or invalidate a verification run.

## Features

- [Boot report](./boot-report.md) covers the startup banner and the chip, flash, and PSRAM report a user sees after power-on.
- [Heartbeat](./heartbeat.md) covers the periodic liveness line and its free-heap figure.
- [Debug console](./debug-console.md) covers the M1-03 `td> ` console and its `@td` JSON replies, driven by `$TD serial cmd`.
- [Display](./display.md) covers the M1-04 first frame, the eight-bar test screen, and the `screenshot`, `perf fps`, and `backlight` console commands, driven by `$TD serial screenshot` and `$TD serial cmd`.
  From M1-06 on, the test screen is gone, so its bar recipes hold at M1-04 and M1-05 only.
- [Touch](./touch.md) covers the M1-05 GT911 touch input, the injected taps and swipes, the `touch` and `ui` console commands, and the test screen's `Taps: N` button, driven by `$TD serial cmd` and `$TD serial capture`. Its operator-only checks are marked there.
  From M1-06 on, the `Taps: N` button and its aim point are gone, and the tiles of [UI pages](./ui-pages.md) take their place.
- [UI pages](./ui-pages.md) covers the M1-06 deck of two swipeable pages of tiles with page dots, the `ui state`, `ui goto`, `ui tile`, and `ui hold` console commands, and the page transition timing, driven by `$TD serial cmd`, `$TD serial screenshot`, and `$TD serial capture`. Its operator-only checks are marked there.
- [F-Keys page](./fkey-page.md) covers the M1-10 `F-Keys` tiles that send keys through the key engine, the three looks of the modifier tiles, the one-shot clear on a page swipe, and the `tile_count` and `name` fields of `ui state` and `ui tile`, driven by `$TD serial cmd`, `$TD serial screenshot`, `$TD serial capture`, and `$TD serial keys`.
- [Keys](./keys.md) covers the M1-09 key engine, its one-shot and locked modifiers, the console key log and record, and the `keys` console command, driven by `$TD serial cmd`, `$TD serial capture`, and `$TD serial keys`.
- [Media page](./media-page.md) covers the M1-11 `Media` tiles that send consumer-control usages through the key engine, the icon over a caption on each tile, and the check that a locked modifier never goes out with a media key, driven by `$TD serial cmd`, `$TD serial screenshot`, `$TD serial capture`, and `$TD serial keys`.
- [Idle](./idle.md) covers the M1-12 idle screen-off, the wake touch that the gate eats, the modifiers it clears, and the `power` console command, driven by `$TD serial cmd`, `$TD serial capture`, `$TD serial keys`, and `$TD serial screenshot`. Its operator-only checks are marked there.
- [App link](./app-link.md) covers the M2-01 SLIP-framed app link that shares the console's serial port, the `link state` console command, and the `hello`, `ping`, and `link.state` messages, driven by `$TD link` and by `$TD link scenario` when a lane needs the console and the link on one port handle.
- [Storage](./storage.md) covers the M2-03 LittleFS volume at `/storage`, the atomic file swap, the persisted settings, and the `store` and `settings` console commands, driven by `$TD serial cmd`, `$TD serial capture`, and `$TD serial store`.
- [Layouts](./layout.md) covers the M2-04 layout file the deck draws its pages, tiles, icons, and actions from, the built-in fallback, and the load log lines, driven by `$TD serial cmd`, `$TD serial store`, and `$TD link`.
- [Widgets](./widgets.md) covers the M2-06 `widget.set` and `dashboard.set` messages that fill widget tiles and dashboard pages, their clamps, and the offline dimming, driven by `$TD link` and `$TD serial screenshot`.
- [Recovery hold](./recovery.md) covers the M2-08 10-second hold, the `Release to reset` band, the marker boot onto the built-in pages with stored files kept, leaving recovery, and the `recovery` console command, driven by `$TD serial cmd`, `$TD serial screenshot`, `$TD serial keys`, and `$TD serial store`.
- [Firmware update](./update.md) covers the OTA transfer, confirmation, rollback, and recovery bundles.
- [Radio build RAM floors](./radio-heap.md) covers the radio build's steady and boot internal-RAM floors (24,576 and 16,384 bytes, #313), driven by `$TD serial capture` on a radio build in a build directory of its own.

## Not yet mapped

Touch input is mapped in [Touch](./touch.md), and the harness injects touches with `touch tap` and `touch swipe`. The key engine is mapped in [Keys](./keys.md), its reports go to the console key log, and the `F-Keys` tiles that feed it are in [F-Keys page](./fkey-page.md). The idle screen-off is mapped in [Idle](./idle.md), and only a person at the deck can see the backlight itself. The firmware has no standalone USB keyboard or media-key sink yet, and the app types the deck's mirrored keys instead. When a USB HID sink lands, add its feature file here (`/maintain-verification-skill`).
