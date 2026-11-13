# Display

From M1-04 on, the firmware brings up the panel and draws an eight-bar test screen with a TouchDeck label through LVGL. A user sees the lit panel, and a developer or the harness can switch the backlight, measure the render rate, and read back what was drawn, all over the debug console.
From M1-06 on, the deck of tiles in [UI pages](./ui-pages.md) replaces the test screen, so the bar details below hold at M1-04 and M1-05 only.

## Sub-features

- `display-first-frame` logs `display: first frame` after the boot banner, once the first frame is drawn.
- `display-test-screen` draws eight vertical bars and the TouchDeck label. Left to right the bars are RGB565 `0xFFFF` white, `0xFFE0` yellow, `0x07FF` cyan, `0x07E0` green, `0xF81F` magenta, `0xF800` red, `0x001F` blue, and `0x0000` black. Bar i spans x `128*i` to `128*i+127`, and its centre pixel is `(128*i+64, 300)` at scale 1 and `(32*i+16, 75)` at scale 4.
  - **From M1-06 on.** There is no test screen, no bars, and no TouchDeck label. The panel shows the deck from [UI pages](./ui-pages.md), so the bar colours and centres above hold at M1-04 and M1-05 only.
- `display-screenshot` answers `screenshot [1|2|4]` with LVGL's render of the active screen as RGB565. Scale N keeps every Nth pixel, the top-left of each NxN block.
- `display-fps` answers `perf fps <seconds>` with the frame count and rate after the measurement ends. The reply is late and the console stays usable meanwhile.
  - **From M1-06 on.** `perf fps` redraws the whole deck on every tick, so the `fps` it reports is the deck's. Report it with that context, and do not compare it with the bars' figure of M1-04 without saying so.
- `display-backlight` answers `backlight on` and `backlight off` with the state it set.

## How to get to it (user POV)

- Power the board or press RESET. The panel lights and shows the test screen.
  - **From M1-06 on.** The panel shows the deck on page 0, `F-Keys`, and not the test screen.
- Open a serial terminal on the board's port and type `screenshot 4`, `perf fps 5`, `backlight off`, or `backlight on` at `td> `.
- Run `$TD serial screenshot` or `$TD serial cmd "<line>"`, which send the same lines and print the reply as JSON.

## Driving it with touchdeck.ps1

Preconditions:

- The build under test is M1-04 or later, and `$TD serial doctor` exits `0` with `build_matches` true.
- Evidence directory `E=<absolute path>/.verify/<YYYYMMDD-HHMMSS>-display` is new, and Doctor's JSON is saved there as `doctor.json`.
- The backlight is on. A previous run may have left it off, so run `$TD serial cmd "backlight on"` first.

- **First frame.** Run `$TD serial capture --out $E/boot --seconds 10 --expect "touchdeck: TouchDeck firmware starting" --expect "display: first frame"`. Exit `0`, with the `display: first frame` line after the banner in `$E/boot/serial.log`.
- **Test screen at full size.** Run `$TD serial screenshot --out $E/full.png --raw $E/full.rgb565 > $E/full.json`. Exit `0`, and `full.json` has `ok` and `crc_ok` true, `width` 1024, `height` 600, `format` `rgb565`, `bytes` 1228800, and `lines` 1600. Read the bar centres from the raw file with `$raw = [IO.File]::ReadAllBytes("$E/full.rgb565"); 0..7 | % { $o = (300*1024 + 128*$_ + 64)*2; '{0:X4}' -f ($raw[$o] + 256*$raw[$o+1]) }`. The eight values are `FFFF FFE0 07FF 07E0 F81F F800 001F 0000`.
  - **From M1-06 on.** The command and the `full.json` checks hold unchanged. There are no bars, so the read-back above returns deck pixels and not the eight values, which hold at M1-04 and M1-05 only. Read the deck's pixels with the recipes of [UI pages](./ui-pages.md).
- **Look at it.** View `$E/full.png`. It shows the eight bars in that order and the TouchDeck label. In the PNG the bar centres are (255,255,255), (255,255,0), (0,255,255), (0,255,0), (255,0,255), (255,0,0), (0,0,255), and (0,0,0), and `r5=r8>>3`, `g6=g8>>2`, `b5=b8>>3` gives the RGB565 values back exactly.
  - **From M1-06 on.** `full.png` shows the deck instead: `F-Keys` at the top left, sixteen tiles, and two page dots. The bar centre values above hold at M1-04 and M1-05 only. [UI pages](./ui-pages.md) lists the deck's pixel values.
- **Smaller scales.** Run `$TD serial screenshot --scale 4 --out $E/s4.png --raw $E/s4.rgb565 > $E/s4.json`, then the same with `--scale 2` and `s2`. Exit `0`. Scale 4 gives `width` 256, `height` 150, `bytes` 76800, `lines` 100, with the bar centres at `(32*i+16, 75)`. Scale 2 gives 512 by 300, `bytes` 307200, `lines` 400, with the centres at `(64*i+32, 150)`. Both read the same eight values as scale 1.
  - **From M1-06 on.** The sizes, byte counts, and line counts hold. The eight bar values hold at M1-04 and M1-05 only, because there are no bars.
- **Repeated shots.** Run `$TD serial screenshot --scale 4 --count 3 --out $E/rep.png > $E/rep.json`. Exit `0`, the files are `rep-1.png` to `rep-3.png`, and every `shots` entry has `crc_ok` true. While nothing on screen changes, the three `crc32` values are equal.
  - **From M1-06 on.** The command and the checks hold unchanged. The pictures show the deck, and the three `crc32` values are equal while the deck is at rest, which `ui state` shows as `animating` false and an empty `pressed_tile`.
- **Frame rate.** Run `$TD serial cmd "perf fps 5" --timeout 8 > $E/fps.json`. Exit `0`, `ok` true, `seconds` 5, `frames` above 0, `elapsed_ms` near 5000, and `fps` near `frames*1000/elapsed_ms`. Report the `fps` figure. This recipe sets no target.
  - **From M1-06 on.** `fps` comes from a full redraw of the deck, not the bars, so report it with that context. The bound of 16 that the plan sets for `perf fps 5` at M1-04 was set on the bars. The plan's M1-06 rule is 15 frames per second during a page transition, which [UI pages](./ui-pages.md) measures from `ui state`. The figures that the older recipes quote stay as written.
- **Screenshots during a measurement.** Run `$TD serial screenshot --scale 4 --count 3 --background "perf fps 10" --out $E/during.png > $E/during.json`. Exit `0`, `background_ok` true, `background` is the perf reply with `seconds` 10 and `frames` above 0, and every shot has `crc_ok` true. When the three `seconds` values add up to under 10, the shots finished while the measurement was still running, so the console stayed usable.
- **Repeated boots.** Run `$TD serial capture --out $E/reboots --seconds 50 --no-reset --send "reboot" --send-after 1 --send-times 5 --send-every 9 --expect "touchdeck: TouchDeck firmware starting" --expect "display: first frame" --expect "touchdeck: TouchDeck firmware starting" --expect "display: first frame" --expect "touchdeck: TouchDeck firmware starting" --expect "display: first frame" --expect "touchdeck: TouchDeck firmware starting" --expect "display: first frame" --expect "touchdeck: TouchDeck firmware starting" --expect "display: first frame"`. Exit `0` only when five banners each have their own first frame after them. Then read the device timestamps `I (<ms>)` of each banner and first frame in `$E/reboots/serial.log`. The script does not check the gap, so the plan's 1500 ms bound is checked by hand.
- **Serial transcript during load.** Add `--log $E/during.log` to the measurement run above. `during.log` is the console's text for the session, without the image lines, so a warning or error from the display shows up there.
- **Early command.** Run `$TD serial capture --out $E/early --seconds 8 --send "screenshot 4" --send-after 0 --send-times 12 --send-every 0.08`. A request that reaches the console before `display: first frame` gets `{"cmd":"screenshot","ok":false,"error":"display not ready"}`, and `serial.log` has no `Guru Meditation`. Later requests stream an image. A run where every request landed after the first frame is not a failure, only a missed window.
- **Backlight.** Run `$TD serial cmd "backlight off"`. Exit `0`, and the reply is `{"cmd":"backlight","ok":true,"on":false,"output":26}`. Run `$TD serial cmd "backlight on"`. Exit `0`, and the reply has `"on":true`. The replies prove the firmware set the output. Whether the panel went dark is for the operator to see, so report it as not verified unless they confirmed it.
- **Proof.** Keep `$E/doctor.json`, `$E/boot/`, the PNG, raw, and JSON files, and name in the report what only the operator can confirm: drift, flicker, tearing, and backlight state.

## Gotchas

- A screenshot is LVGL's render of the active screen. It cannot show drift, flicker, tearing, or backlight state. Report those as not verified unless the operator looked at the real panel.
- From M1-04 on, `alive 0` prints after the display has started, about 0.7 seconds after the banner instead of right after it.
- `display: first frame` prints during boot, and USB-Serial/JTAG drops output printed before the port opens. Let `capture` do the reset, and don't use `--no-reset` for it.
- The screenshot stream blocks the console until its trailer arrives, and a full-size shot takes several seconds. `$TD serial cmd "screenshot"` returns only the header and leaves the stream in flight, so always use `$TD serial screenshot`.
- A screenshot taken while `perf fps` runs may wait several frames. Don't take the `fps` of a run that overlapped screenshots as the idle figure. Run `perf fps` alone for that.
- `perf fps` replies when the measurement ends. Give `cmd` a `--timeout` of the seconds plus 3.
- `help` lists all eight commands from M1-04 on, the five M1-03 commands and then `screenshot`, `perf`, and `backlight`. From M1-05 on it also lists `ui` and `touch`.
- From M1-05 on, a rounded `Taps: N` button sits at the bottom centre of the test screen, over x 292 to 731 and y 440 to 539. The eight bar centres stay uncovered, so the pixel values above read the same, and `full.png` shows the button as well as the bars. [Touch](./touch.md) covers it.
  - **From M1-06 on.** The button is gone with the test screen, and the tiles of [UI pages](./ui-pages.md) replace it.
- Scale N keeps one pixel per NxN block and averages nothing, so detail thinner than N pixels can vanish at scales 2 and 4. Use scale 1 for pixel-exact claims. The bar centres read the same at every scale.
- Pixels are little-endian, low byte first, so `0xF800` red is the bytes `00 F8`.
- A label glyph can sit on a bar. If a centre pixel does not read as its bar colour, view `full.png` before calling the screen wrong, because the probe may have landed on a glyph.
  - **From M1-06 on.** There are no bars. The gotchas above about bar centres at every scale and a label glyph on a bar hold at M1-04 and M1-05 only.
- `--count N` writes `base-1.png` to `base-N.png` and never `base.png`. A shot that fails a check writes no file, so use a new evidence directory.
