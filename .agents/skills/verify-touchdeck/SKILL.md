---
name: verify-touchdeck
description: Build, flash, and drive the TouchDeck ESP32-S3 firmware on the real Waveshare ESP32-S3-Touch-LCD-5B board over its USB-Serial/JTAG port, inject touches, and capture serial-log and screenshot evidence that a firmware change works. Use when you need to prove a firmware behavior on hardware, check what the connected board is running, or reflash it for a verification run.
---

# Verify TouchDeck firmware

TouchDeck is ESP-IDF firmware for one physical board: a Waveshare ESP32-S3-Touch-LCD-5B on USB. An agent reaches it through the board's single USB-C port (native USB-Serial/JTAG, VID `303a` PID `1001`) and observes two things. The **serial log** shows what the firmware prints. From M1-04 on, a **screenshot** shows LVGL's render of the active screen, which is what the firmware drew. A screenshot does not show drift, flicker, tearing, or backlight state. Only the operator looking at the real panel can show those, so never report them as verified from a screenshot.

From M1-05 on, an agent can also inject touches. An injected touch enters at LVGL's input device, so it proves the UI logic and LVGL's input handling, such as a tap that raises the button counter or a swipe that raises a gesture. It does not exercise the GT911 controller or the glass. A real finger on the glass, orientation, mirroring, feel, and accuracy only the operator can show, so report them as pending operator. The USB keyboard is not built yet. Never report a touch behavior that needs a finger as verified from an injected touch, serial logs, or screenshots, and never report a screen behavior as verified from serial logs alone.

All commands run from the repo root. `$TD` below means:

```
pwsh -NoProfile -File .claude/skills/verify-touchdeck/scripts/touchdeck.ps1
```

`touchdeck.ps1` activates the selected ESP-IDF install (from EIM's default `C:\Espressif\tools\eim_idf.json`, override with `$env:TOUCHDECK_EIM_JSON`) and then runs `idf.py` (`$TD idf ...`), the serial harness `scripts/touchdeck_serial.py` (`$TD serial ...`), esptool (`$TD esptool ...`), the unit tests (`$TD unit`), or the framed app link `scripts/touchdeck_link.py` (`$TD link ...`). It works from Bash or PowerShell, with no activated shell needed.

For framed messages or mixed console and link traffic, read [features/app-link.md](features/app-link.md). It defines `link hello`, `ping`, `state`, `raw`, the PC `selftest`, and `link scenario <file>`. The scenario runner owns one port handle for timed sends, logs, and sequence-matched replies. Use it for milestone 2 lanes 6, 8, and 9.

For widget tiles, dashboard pages and offline dimming, read [features/widgets.md](features/widgets.md). `link hold` keeps the link online, `link widget-set` and `link dashboard-set` send the app's updates, and each can take a screenshot on the same port while the link is online.

### Machine settings

The scripts carry no machine-specific values. Set these in your shell or user environment when the defaults don't fit:

| Variable | Default | Used for |
| --- | --- | --- |
| `TOUCHDECK_PORT` | the single Espressif USB-Serial/JTAG port | the board's serial port, such as `COM5` or `/dev/ttyACM0`. `$TD` passes it to `idf`, `esptool`, `unit` and `serial` unless you pass `-p`/`--port` yourself |
| `TOUCHDECK_ROOT` | the checkout the script lives in | the checkout to build and test |
| `TOUCHDECK_EIM_JSON` | EIM's default `eim_idf.json` | the ESP-IDF install list |
| `TOUCHDECK_FACTORY_BIN` | none | the full-flash backup of the Waveshare factory demo (see Cleanup) |

## Isolation

There is one board and one port. Two runs cannot drive it at once, and every flash replaces what the user is running. Before you flash:

1. Run Doctor. If `port_free` is false, another program (usually the VS Code ESP-IDF monitor) holds the port. Ask the user to close it. Never kill VS Code or another process you didn't start.
2. If `flashed_app.project_name` is not `touchdeck`, the board runs someone else's firmware (for example the Waveshare factory demo, `rtc_pcf85063`). Doctor then skips its serial check and exits `2`. Stop and ask the user before flashing.

## Launch

1. Build: `$TD idf build`. Done when it prints `Project build complete`. `sdkconfig` is committed, so no `set-target` is needed. If the build directory is stale or broken, run `$TD idf fullclean` and build again.
2. Flash: `$TD idf flash`. Done when it prints `Done` after one line per image: `Hash of data verified.` (written) or `'<image>' ... verified.` (unchanged, so esptool skipped the write and checked flash instead). This writes the bootloader, partition table, and app, and leaves other flash regions alone. Never run `erase-flash` without the user's go-ahead. Future firmware will keep user layouts in flash.
3. Ready: run Doctor and require `"ok": true`. That means the flashed app is byte-for-byte this build, and the firmware printed its banner and a heartbeat after reset.

Teardown: nothing runs on the host after a command returns. The board keeps running what you flashed, which is the expected end state.

## Doctor

```
$TD serial doctor            # --port <port> to pin the port; --seconds N to wait longer
```

Read-only. It reads 256 bytes of flash and resets the board, but writes nothing. It prints JSON and exits `0` when ready, `1` when the port is missing or busy, and `2` when the board is not running this build or never showed a heartbeat.

| Field | Meaning |
|---|---|
| `port`, `port_free` | Espressif USB-Serial/JTAG port found and openable by us |
| `local_build` | app descriptor of `build/touchdeck.bin` (git `version`, build `time`/`date`, `idf_ver`, `elf_sha256`) |
| `flashed_app` | app descriptor read from the app slot the partition table at `0x8000` names (`ota_0`, else `factory`) |
| `build_matches` | `elf_sha256` of flash equals the local build |
| `banner_seen`, `alive_seen` | after reset, saw `touchdeck: TouchDeck firmware starting` and `touchdeck: alive N` |

`version` ending in `-dirty` means the build included uncommitted changes. That's normal mid-task.

## Drive

```
$TD serial capture --out <evidence dir> --seconds <N> --expect "<regex>" [--expect "<regex>" ...] [--stop-when-matched] [--no-reset] [--send "<line>" ...] [--send-after <S>] [--send-times <N> --send-every <S>]
$TD serial ports              # list Espressif USB devices
$TD serial cmd "<line>"       # M1-03 and later: run one console command
$TD serial screenshot --out <abs path>/shot.png   # M1-04 and later: capture the display, see Screenshot
$TD serial keys --out <abs path>/record.json      # M1-09 and later: read the key record, see Keys
```

`capture` resets the board (unless `--no-reset`), records every line with a time offset, and checks the `--expect` regexes **in order**. Each must match a later line than the one before. Exit `0` means all matched. `--send` writes console lines once, `--send-after` seconds into the capture (default 1), so one capture can hold a command and what follows it, such as `reboot`. `--send-times N --send-every S` writes the `--send` lines N times in total, S seconds apart, so one capture can hold several reboots. Each send is marked in `serial.log` as `>> sent` or `>> send failed`. The port is exclusive, so another process cannot send during a capture. Regexes match the log text after the `I (1234) ` prefix, so anchor on the tag, e.g. `touchdeck: PSRAM: \d+ KB`. The exact recipes for each feature live in [features/README.md](features/README.md).

## Console commands

Firmware from M1-03 on runs a debug console on USB-Serial/JTAG with the prompt `td> `. Every command answers with one line, `@td ` followed by one JSON object that holds `cmd`, `ok`, and the command's own fields. The commands are `help`, `version`, `heap`, `uptime`, and `reboot`, from M1-04 also `screenshot`, `perf`, and `backlight`, from M1-05 also `touch` and `ui`, from M1-09 also `keys`, and from M1-12 also `power`. An unknown command answers `ok` false with an `error`. The console splits a line into words with `esp_console_split_argv`. It splits on spaces only, honors double quotes, and keeps an escaped backslash, quote, or space, so a tab stays inside its word and `"no such" cmd` runs the word `no such`. It queues up to 2047 bytes of input while a command runs. Past that it drops one read of up to 64 bytes and then every byte through the next CR or LF, so a line that a drop cut never runs. It answers `{"cmd":"console","ok":false,"error":"input overflow","dropped_bytes":N}` before its next prompt. `dropped_bytes` counts every byte it discarded. For the bytes of answered lines plus `dropped_bytes` to equal the bytes sent, the sum is exact only when no line is blank and every line ends in one CR or one LF. With CR LF endings it can be off by one byte per overflow, short or over. One loss can print several `input overflow` lines, so add every `dropped_bytes`. The USB driver underneath can still lose a 64-byte packet uncounted if `td_rx` is starved, so a shortfall is a finding, not proof of a firmware counting bug.

From M1-06 on, `ui` has subcommands: `ui state`, `ui goto <page>`, `ui tile <id>`, and `ui hold <x> <y> <ms>`. A `ui` line with an unknown subcommand answers `ok` false with a usage text. `help` still lists the same ten names in the same order. [features/ui-pages.md](features/ui-pages.md) covers them.

```
$TD serial cmd "version"      # [--port <port>] [--timeout 3] [--repeat N] [--interval S] [--time]
```

`cmd` opens the port without resetting the board, drops stale input, writes the line, and ignores the echo and log lines until the `@td` reply whose `cmd` is the line's first word (parsed the way the console parses it), or its first 63 characters, which is all the firmware echoes. It prints that JSON on stdout and exits `0` when `ok` is true, `1` when `ok` is false, and `2` when no port is found, the port is unusable, or no reply came in time. `--repeat N` sends the line N times in one session, `--interval S` waits S seconds between them, and `--time` prints each round trip to stderr as `rtt_ms`.

Firmware before M1-03 has no console, so `cmd` exits `2` on it. `doctor`, `capture`, and `ports` work on every link.

Firmware from M1-04 on adds three commands for the display, and `help` lists them after the five, in this order:

```
backlight on|off     @td {"cmd":"backlight","ok":true,"on":false,"output":26}
perf fps <seconds>   @td {"cmd":"perf","ok":true,"seconds":S,"frames":F,"elapsed_ms":E,"fps":N}
screenshot [1|2|4]   @td {"cmd":"screenshot","ok":true,"phase":"start","scale":S,"width":W,"height":H,"format":"rgb565","bytes":B,"lines":L}
                     L lines of  @tdb <base64>
                     @td {"cmd":"screenshot","ok":true,"phase":"end","bytes":B,"crc32":N}
```

- Until the display has started, about 0.55 s after the banner, `screenshot` and `perf` answer `{"cmd":"screenshot","ok":false,"error":"display not ready"}` instead of running.
- `backlight` drives the backlight output and reports the state it set in `on`. `output` is the CH422G output byte after the change, 26 with the backlight off and 30 with it on.
- `perf fps <seconds>` replies late. The prompt returns at once and the console stays usable. The one reply line arrives when the measurement ends. Run it as `$TD serial cmd "perf fps 10" --timeout 13`, the seconds plus 3, so `cmd` waits long enough.
- `screenshot` streams the 1024x600 panel as RGB565. Scale N divides the size by N and keeps every Nth pixel in both axes, the top-left pixel of each NxN block. Each `@tdb` line holds 768 raw bytes as 1024 base64 characters. Only the last line may be shorter, with `=` padding, and every line decodes on its own. A pixel is 2 bytes, little-endian, row-major with no padding. `crc32` is `zlib.crc32` of all decoded bytes in stream order. A failure before streaming is a single `@td` line with `ok` false and an `error`. A failure mid-stream ends with the `end` line carrying `ok` false and an `error`. Use `$TD serial screenshot`, not `cmd`.

Firmware from M1-05 on adds the `ui` and `touch` commands, and `help` lists them after the display's three, `ui` first. Before the hardware is up, a well-formed `touch` command answers `{"cmd":"touch","ok":false,"error":"touch not ready"}` and `ui state` answers `{"cmd":"ui","ok":false,"error":"display not ready"}`. A malformed `touch` line gets its usage error first:

```
touch tap <x> <y>     @td {"cmd":"touch","ok":true,"op":"tap","x":512,"y":490,"samples":2,"deliver_us":19873}
touch swipe <x1> <y1> <x2> <y2> <ms>
                      @td {"cmd":"touch","ok":true,"op":"swipe","samples":32,"deliver_us":318202}
touch log on|off      @td {"cmd":"touch","ok":true,"op":"log","on":true}
touch stats           @td {"cmd":"touch","ok":true,"op":"stats","address":"0x5D","polls":N,"frames":N,"pressed_frames":N,"errors":N,"injected":N,"rejected":N,"queued":N,"x":N,"y":N,"pressed":false,"contact_frames":N,"contact_ms":N,"contact_hz":N,"uptime_ms":N}
ui state              @td {"cmd":"ui","ok":true,"taps":N,"button_x":292,"button_y":440,"button_w":440,"button_h":100}
```

- `touch tap` and `touch swipe` inject touches at LVGL's input device. `x` is 0 to 1023, `y` is 0 to 599, and `ms` is 1 to 2000, rounded down to whole 10 ms steps with at least one step. They reply late, when the injected samples have reached LVGL and LVGL has run one more read, so the click or gesture they cause, if any, has been dispatched. `deliver_us` is the time from queueing to the moment LVGL took the last sample, the release, which raises the click. A tap takes up to about 20 ms and a swipe takes about `ms` plus 10 to 20 ms, so the default `cmd` timeout of 3 seconds covers both. They block the console until they reply.
- A failure answers `ok` false with an `error`. The errors are `usage: touch tap <x> <y>` and similar usage texts for the other forms, `out of range`, `finger down` while a real finger is on the glass, `queue full`, and `not consumed in time` with the samples still queued. `finger down` and `queue full` queue nothing.
- `touch log on` prints each fresh real GT911 report as `touch: x=<x> y=<y> pressed=<0|1>`, and each LVGL swipe gesture, real or injected, as `touch: gesture left|right|up|down`. A report with no finger prints `pressed=0` with the last coordinates. The log is off after boot. `ui: click taps=N` prints for each click on the test screen's button, with the log on or off.
- `touch stats` replies with these fields. `address` is the GT911 I2C address that answered at boot. `polls` counts status polls, one per LVGL read, so 100 per second while LVGL keeps up. `frames` counts polls that found a fresh controller report, and `pressed_frames` counts those with a finger down. `errors` counts I2C read failures. `injected` counts injected samples delivered to LVGL since boot, `rejected` counts injected gestures refused because the queue was full, and `queued` counts injected samples still waiting. `x`, `y`, and `pressed` are the last sample LVGL took, real or injected. `contact_frames`, `contact_ms`, and `contact_hz` describe the latest finger contact. `contact_hz` is fresh GT911 reports per second while the finger was down. It stays after the finger lifts, and it counts only real reports.
- `ui state` reports `taps`, the clicks on the test screen's button since boot. The button covers x 292 to 731 and y 440 to 539, with its centre at (512, 490). The eight bar centres that the display recipe reads stay uncovered.
- From M1-06 on, the test screen, its button, and the bars are gone, and `ui state` replies with `page`, `pages`, `title`, `pressed_tile`, `taps`, `animating`, `transitions`, `transition_ms`, `transition_frames`, `last_ms`, `last_frames`, `dot_x`, `dot_pitch`, `dot_y`, `dot_active`, `dot_idle`, and `glow`, and from M1-10 on `tile_count`, with no `button_x`, `button_y`, `button_w`, or `button_h`. `taps` counts clicks on tiles. A click logs `ui: click taps=N tile=<id>`, which a regex for `ui: click taps=N` still matches. A page slide that ends logs `ui: page shown page=N ms=M frames=F`, a bounce that ends logs `ui: bounce done page=N ms=M frames=F`, and a slide that a new touch cut short logs `ui: slide cut page=N ms=M frames=F`. `perf fps` measures a full redraw of the deck, not the bars.

The recipes for these commands are in [features/touch.md](features/touch.md).
From M1-06 on, the recipes for the deck, `ui goto`, `ui tile`, `ui hold`, and the full `ui state` are in [features/ui-pages.md](features/ui-pages.md).
From M2-04 on, the deck draws its pages from a layout file, and `help` lists `layout` right after `ui`. `ui tile` also replies with `action`, the tile's action type, and `icon`, the empty string when the tile has none. `layout state`, `layout reload`, and `layout use builtin|storage` read and change the layout on screen. Their recipes are in [features/layout.md](features/layout.md).

Firmware from M1-09 on adds the `keys` command, and `help` lists it after `touch`. It drives the key engine, which decides the key and modifier reports that would go to the PC. In milestone 1 no USB keyboard exists, so every report goes to the console key log instead, and nothing reaches the PC. A report is `kbd <mods> [<keys>]` or `con <usage>` in the plan's notation. `mods` is the HID modifier byte, Ctrl 1, Shift 2, Alt 4, Win 8, and `keys` lists keyboard usages, F1 to F12 at 58 to 69 and F13 to F24 at 104 to 115. `kbd 0 []` is the all-released report and `con 0` the empty consumer report. `<usage>` is a decimal usage or `F1` to `F24`.

```
keys tap <usage>        @td {"cmd":"keys","ok":true,"op":"tap","usage":106,"reports":2,"t":T,"sink_us":U}
keys consumer <usage>   @td {"cmd":"keys","ok":true,"op":"consumer","usage":205,"reports":2,"t":T,"sink_us":U}
keys mod ctrl|shift|alt|win
                        @td {"cmd":"keys","ok":true,"op":"mod","mod":"shift","state":"ONE_SHOT","t":T}
keys clear [all]        @td {"cmd":"keys","ok":true,"op":"clear","all":false}
keys state              @td {"cmd":"keys","ok":true,"op":"state","ctrl":"OFF","shift":"OFF","alt":"OFF","win":"OFF","seq":N,"log":false,"recording":false}
keys log on|off         @td {"cmd":"keys","ok":true,"op":"log","on":true}
keys record start|stop  @td {"cmd":"keys","ok":true,"op":"record","recording":true,"count":0,"dropped":0}
keys record dump        @td {"cmd":"keys","ok":true,"op":"record","phase":"start","count":N,"dropped":D}
                        N lines of  @tdr {"seq":S,"t":T,"type":"kbd","mods":M,"keys":[...]}  or  @tdr {"seq":S,"t":T,"type":"con","usage":U}
                        @td {"cmd":"keys","ok":true,"op":"record","phase":"end","count":N}
keys fuzz <n> <seed>    @td {"cmd":"keys","ok":true,"op":"fuzz","events":N,"seed":S,"reports":R,"violations":0,"elapsed_us":E}
```

- `tap` sends the key with every active modifier and then the all-released report, and returns one-shot modifiers to `OFF`. `consumer` sends the usage and then the empty consumer report, and leaves every modifier as it is. `reports` is the count emitted, always 2. `t` is the engine's time of the event in milliseconds since boot, and `sink_us` is the time from the event to the sink taking the last report.
- `mod` taps a modifier. `OFF` goes to `ONE_SHOT`, a second tap within 300 ms of the first goes to `LOCKED`, and a tap on `ONE_SHOT` after 300 ms or on `LOCKED` goes to `OFF`. The reply carries the new `state` and `t`, so two replies tell how far apart the taps were by the board's clock.
- `clear` returns one-shot modifiers to `OFF` and keeps locked ones, and `clear all` returns every modifier to `OFF`. Neither sends a report.
- `state` reports the four modifiers as `OFF`, `ONE_SHOT` or `LOCKED`, `seq`, the reports since boot, and whether the log and the record are on.
- `log on` prints every report as `keys: {"seq":N,"t":T,"type":"kbd","mods":M,"keys":[...]}` or `keys: {"seq":N,"t":T,"type":"con","usage":U}`, a log line that a redrawing display can drop. The log is off after boot and stays on until `keys log off` or a reset.
- `record start` empties the record and starts it. The record keeps 2048 reports in PSRAM and counts the ones that did not fit in `dropped`. `record stop` stops it. Both reply with `recording`, `count` and `dropped`. `record dump` streams the record like `screenshot` does, so use `$TD serial keys`, never `cmd`, which returns the `start` reply and leaves the stream in flight. A dump while recording reads the count once and streams those entries.
- `fuzz` runs `n` random events, 1 to 100000, through the pure reducer on a state of its own with a checking sink of its own, so the engine on the board, the log and the record see nothing. `violations` counts steps that left a key down, that is a down report with no release after it in the same step, and reports that carried a modifier with no key. The reply has `ok` false with `"error":"violations"` when it is above 0. The same `seed` gives the same events.
- A malformed `keys` line gets a usage text with `ok` false. Before the engine has started, which never happens after boot in practice, it answers `keys not ready`.

The recipes for these commands are in [features/keys.md](features/keys.md). From M1-11 on, the recipes for the `Media` tiles that feed the engine are in [features/media-page.md](features/media-page.md).

Firmware from M1-12 on adds the `power` command, and `help` lists it after `keys`, twelve names in all. It reads the idle gate that turns the backlight off after a timeout with no touch, 600 seconds by default, and eats the first touch on the dark screen, real or injected, so that touch only wakes the screen. The gate reads lit or dark from the CH422G output byte, the same byte `backlight` writes, so after `backlight off` the next touch is a wake, and `backlight on` lights the screen without eating anything.

```
power state                 @td {"cmd":"power","ok":true,"op":"state","state":"ACTIVE","idle_timeout_s":600,"idle_ms":N,"swallowing":false,"swallowed":0,"off_count":0,"wake_count":0,"wake_us":0,"errors":0,"output":30}
power idle-timeout <s>      @td {"cmd":"power","ok":true,"op":"idle-timeout","idle_timeout_s":S}
```

- `state` is `ACTIVE` when the backlight bit is set and `OFF` when it is clear, with `output` 30 or 26. `idle_ms` counts from the last touching sample or from the last `idle-timeout`, whichever is later. `swallowing` is true only while an eaten contact is open. `swallowed` counts dropped samples since boot, the wake press and the closing release included, so a tap on the dark screen adds 2 and a swipe adds its `samples`. `off_count` and `wake_count` count the gate's own backlight writes, `wake_us` is the latest wake's time from the wake sample reaching the gate to the backlight write returning, and `errors` counts failed writes.
- `idle-timeout` takes 1 to 86400 seconds, restarts the idle clock, and is not stored, so a reset restores 600. Set while the screen is dark, it keeps the screen dark.
- The screen-off clears every key modifier first and logs `power: backlight off`. A wake logs `power: backlight on`. A failed write logs `power: backlight off write failed: <err>` or the `on` form as a warning.
- A malformed `power` line gets `usage: power state | idle-timeout <seconds 1-86400>`, a timeout word that is not plain digits gets `usage: power idle-timeout <seconds 1-86400>`, and 0 or more than 86400 gets `out of range`, each with `ok` false. Before the gate has started, which never happens after boot in practice, it answers `power not ready`.

The recipes for these commands, and the timing rules a `$TD` call imposes on them, are in [features/idle.md](features/idle.md).

Firmware from M2-03 on adds `store` and `settings` for the LittleFS volume at `/storage` and the settings file on it, and `help` lists them with `@tds`, the continuation line of a put. `$TD serial store put <path> --from <file>` and `$TD serial store get <path> --out <file>` move a file of 1 to 65536 bytes in `@tds` base64 lines and check its CRC-32 both ways. `$TD serial store decode <serial.log> --out <file>` rebuilds the file from a `store get` stream that a `capture` recorded. `store stat`, `store ls`, `store rm`, and `settings` go through `$TD serial cmd`. The deck holds a put in memory and swaps it in atomically after its last line, so a reset in the middle leaves the volume as it was. [features/storage.md](features/storage.md) has the recipes.

The console may print log lines such as `I (1234) display: ...` and a `td> ` prompt between lines, and `td> ` may sit in front of an `@td` or `@tdb` on the same physical line.

## Screenshot

```
$TD serial screenshot --out <abs path>/shot.png [--scale 1|2|4] [--raw <abs path>/shot.rgb565] [--count N]
                      [--background "<console line>"] [--background-timeout 30] [--timeout 20] [--retries 0] [--log <abs path>/shot.log] [--port <port>]
```

M1-04 and later. Like `cmd`, it opens the port without resetting the board and drops stale input once at the start. It writes `screenshot <scale>`, reads the header, the `@tdb` lines, and the trailer, and checks that the header is `ok`, every line decodes, the decoded byte count equals the header's `bytes` and `width*height*2`, the line count equals `lines`, and `zlib.crc32` of the decoded bytes equals the trailer's `crc32`. Then it writes the PNG. Pass absolute paths, because `$TD` changes directory first.

| Option | Meaning |
|---|---|
| `--scale` | `1` (default, 1024x600), `2` (512x300), or `4` (256x150) |
| `--out` | PNG file, required unless `--raw` is given. With `--count N` above 1 the files are `base-1.png` to `base-N.png` |
| `--raw` | also write the raw little-endian RGB565 bytes, numbered the same way |
| `--count N` | take N screenshots in one serial session |
| `--background "<line>"` | send this console line once before the first screenshot without waiting for its reply. Its reply, matched by the line's first word as `cmd`, is collected whenever it arrives, between shots or after the last, for up to `--background-timeout` seconds (default 30) from sending the line |
| `--timeout` | seconds to wait for each screenshot, from writing the request to the trailer (default 20) |
| `--log` | write every line the console printed except `@tdb`, with time offsets and a `>> sent` mark for each line the harness wrote, to this file. It is the serial transcript of the session, for example to look for `lcd` warnings during a measurement |
| `--retries N` | request the whole screenshot again after a CRC or length failure, up to N times (default 0, so nothing is retried silently). A firmware error or a timeout is never retried |
| `--port` | serial port, default the single Espressif USB-Serial/JTAG port, as for `cmd` |

stdout is one JSON object with `ok`, `port`, `scale`, `width`, `height`, `format`, `bytes`, `lines`, `crc32` (the trailer's), `crc_ok`, `seconds` (from writing the request to the trailer), `bytes_per_s`, `out`, `raw` (when given), `attempts`, and `error` when the shot failed. With `--count` above 1 it holds `ok`, `port`, `scale`, and `shots`, a list of those per-shot objects. With `--background` it also holds `background`, the reply object or null, and `background_ok`. The top-level `ok` is true only when every shot and the background reply are ok. A shot that fails a check writes no file.

Exit `0` when everything is ok. Exit `1` when a check failed (CRC mismatch, short or long data, firmware `ok` false) or the background reply is missing or `ok` false. Exit `2` when the options are unusable, the port is missing or unusable, or no reply came in time, which includes a stream that stalls before its trailer. After a timeout the JSON is still printed with the shots so far, and no further shot is requested, because the board may still be streaming.

The PNG holds the exact pixels. Each RGB565 value expands to `r8=(r5<<3)|(r5>>2)`, `g8=(g6<<2)|(g6>>4)`, `b8=(b5<<3)|(b5>>2)`. Recover the exact RGB565 value from a PNG pixel with `r5=r8>>3`, `g6=g8>>2`, `b5=b8>>3`. Compare colors as RGB565 values, or read `--raw`, not by eye or from a recompressed copy.

To take screenshots while a long command runs, give that command to `--background`:

```
$TD serial screenshot --scale 4 --count 3 --background "perf fps 10" --out <abs path>/during-perf.png
```

This starts a 10 second `perf fps` measurement, takes three shots while it runs, and writes `during-perf-1.png` to `during-perf-3.png`. `background` holds the perf reply.

Timing. A full-size shot moves 1,228,800 bytes as about 1.64 MB of base64 text over USB-Serial/JTAG, which takes several seconds, so keep the default 20 second `--timeout` or raise it. `--scale 2` moves a quarter of the bytes and `--scale 4` a sixteenth. `seconds` and `bytes_per_s` in the output give the measured rate.

## Keys

```
$TD serial keys --out <abs path>/record.json [--log <abs path>/record.log] [--timeout 30] [--port <port>]
```

M1-09 and later. Like `screenshot`, it opens the port without resetting the board and drops stale input once. It writes `keys record dump`, reads the `start` reply, the `@tdr` lines and the `end` reply, and checks that the firmware said `ok`, that the line count equals `count` in both replies, and that every line is JSON with an integer `seq` and `t` and a `kbd` or `con` type. Then it writes `--out` as `{"count":N,"dropped":D,"reports":[...],"brief":[...]}`, where `reports` holds each `@tdr` object in stream order and `brief` the same reports as `kbd <mods> [<keys>]` or `con <usage>`. Pass absolute paths, because `$TD` changes directory first.

stdout is one JSON object with `ok`, `port`, `count`, `dropped`, `lines`, `out`, `brief`, and `error` when a check failed. Exit `0` when every check passed, `1` when one failed or the firmware answered `ok` false, and `2` when the port is missing or unusable or the stream stalled before its `end` reply. A record that fails a check writes no file. `--log` writes every line the console printed except `@tdr`, with time offsets, as `screenshot --log` does.

It reads the record as it is. Run `keys record start` before the events and `keys record stop` after them with `$TD serial cmd`, then `$TD serial keys`. A dump of 2048 reports moves about 170 KB and takes about a second.

## Unit tests

```
$TD unit                      # [--port <port>] [--seconds 30] [--out <evidence dir>]
```

It builds `firmware/test_app`, flashes it with `idf.py -C`, resets the board, and reads serial until Unity prints `N Tests M Failures K Ignored` and `OK` or `FAIL`. Build and flash output goes to stderr. On stdout it prints only a JSON report with `tests`, `failures`, `ignored`, `result`, and the `:FAIL` lines, and exits `0` only when at least one test ran with 0 failures. `serial.log` goes to `--out`.

The test app picks up every component's `firmware/components/<name>/test/` folder. A `test/CMakeLists.txt` there sets `TD_TEST_SOURCES` to its `.c` files, and the test app links the component the folder belongs to. A new component only adds a `test/` folder.

`unit` leaves the test app in the `ota_0` slot, so Doctor then reports `project_name` `touchdeck_test` and exits `2`. It does not reflash the main firmware. Flash the build under test again per the boot recipe before the next lane.

## Evidence

Put every run in `.verify/<YYYYMMDD-HHMMSS>-<feature-id>/` at the repo root (git-ignored). `capture --out` writes:

- `serial.log`: the full timestamped transcript, showing the reset (the action) and every line after it (the resulting state).
- `summary.json`: port, start time, the local build's app descriptor, and each expectation with the line and time it matched.

For a flash-dependent proof, also save the Doctor JSON next to them (`$TD serial doctor > .verify/<run>/doctor.json`), so the evidence shows which build was on the board. For a display proof, keep each screenshot's PNG, its `--raw` file, and its JSON (`> $E/shot.json`) in the run directory. For a touch proof, keep the `ui state` JSON from before and after each injection, the injection replies, and each capture's `serial.log` and `summary.json`. Proof standards:

- Prove behavior on the flashed build. A log from an older image proves nothing, so require `build_matches: true`.
- Assert observable lines, not that the build succeeded.
- A boot-time line only proves the boot path. Behaviors that repeat need two or more occurrences over time.
- When a capture fails, report the failing expectation and the last lines of `serial.log`. Don't loosen the regex to make it pass.

## Cleanup

- Close nothing by name. If you started a background monitor or capture, stop that task by its own ID.
- Leave `.verify/` in place. It is the evidence and cleanup must not remove it.
- Leave the board running the build you flashed and tell the user it's flashed.
- Only when the user asks for the original Waveshare factory demo back, restore the full-flash backup the maintainer keeps outside the repo. `$env:TOUCHDECK_FACTORY_BIN` names it, and its SHA256 is in the `.sha256` file next to it. Check the hash first:
  `$TD esptool write-flash 0 $env:TOUCHDECK_FACTORY_BIN`

## Gotchas

- Logs printed before the port opens are lost on USB-Serial/JTAG. Boot proofs must let `capture` do the reset, and should not use `--no-reset`.
- Before M1-03 the primary console is UART0, which this board wires to the RS485 transceiver, not to USB. What we read is the secondary USB-Serial/JTAG console, so bootloader output may be missing from it while app logs (`I (...) tag: ...`) appear. From M1-03 on the primary console is USB-Serial/JTAG, so the ROM and bootloader lines appear too.
- From M1-03 on the console prints `td> ` before each read. A log line that arrives while it waits starts with that prompt, as in `td> I (4012) touchdeck: alive 1, ...`. `capture` regexes search the line, so they still match.
- When firmware starts using TinyUSB (the planned USB keyboard), the USB-Serial/JTAG port disappears. The VID/PID changes, Doctor will find no port, and flashing needs a human to hold BOOT and tap RESET. Update this skill (`/maintain-verification-skill`) in the same change that adds TinyUSB.
- Unattended ROM recovery needs GPIO19, GPIO20, the USB-Serial/JTAG controller, its clock, and its power left as they are, with no light sleep, deep sleep, eFuse write, or configuration that disables USB-Serial/JTAG. Milestone 2 keeps all of them.
- `idf.py monitor` needs an interactive terminal. Use `capture` instead.
- Replies leave through the USB driver and wait for the host. From M2-01 on, the prompt and typed echo use that same writer. Older firmware routes the prompt and echo through stdio. Log lines still use stdio and can be dropped under display load, so use replies or records when counts matter.
- `capture`, and so `doctor` and `unit`, read the port through the 0.4 s reset, not after it. On the dev PC the port holds only about 12 KB for a reader that is not there, and the chip drops the rest, so before this the unit test app lost its tail once it printed more than that.
- A `screenshot` stream blocks the console for its whole duration. Nothing else answers until the trailer arrives, so don't run `cmd` or a second `screenshot` meanwhile. Never use `cmd` for `screenshot`. It returns the header and leaves the stream in flight.
- `touch tap` and `touch swipe` block the console until LVGL has taken their samples, so run no other `cmd` meanwhile. `touch log on` stays on until `touch log off` or a reset, so turn it off after a recipe that needs it.
- `keys record dump` streams and blocks the console until its `end` reply, like `screenshot`. Never use `cmd` for it. `keys log on` stays on until `keys log off` or a reset. A reset empties the record and returns every modifier to `OFF`.
- In milestone 1 no key reaches the PC. The key engine's only sink is the console key log, so a `keys:` line or a record entry is the proof that a report went out, and nothing on the PC changes.
- From M1-12 on, a board left idle longer than `idle_timeout_s` (600 by default) has its screen off, and the first injected tap, swipe, or hold only wakes it: no tile fires and no record entry appears. Run `$TD serial cmd "power state"` before a touch recipe, and if it says `OFF`, wake the screen with `touch tap 266 128` or set `power idle-timeout 86400` for the run and restore 600 afterward. A screenshot while the screen is dark still shows the deck and proves nothing about the backlight. [features/idle.md](features/idle.md) has the rules.
- A screenshot taken while `perf fps` runs may wait several frames for the display, so raise `--timeout` when you combine them. Don't read the `fps` of a run that overlapped screenshots as the idle figure. Measure it alone.
- Log lines and the `td> ` prompt can interleave with a stream only between its lines, never inside one. The harness skips every line that is not `@td` JSON or `@tdb`, and accepts `td> ` in front of either. A reply also counts after any other text on its line, because a dropped echo can leave `td> he` with no newline in front of it.
- A shot that fails a check writes no file. Use a new evidence directory, so an older PNG with the same name can't pass for it.
- `$TD` changes directory to the checkout under test before it runs the tool, so pass absolute paths for any file argument, such as the image or output file of `$TD esptool ... write-flash` or `read-flash`, or `--out` of `capture` and `unit`.
- Always go through `$TD`. Started from Git Bash, a bare `idf.py` sees `MSYSTEM`, prints `MSys/Mingw is no longer supported`, and **exits 0 without building or flashing**. `touchdeck.ps1` clears `MSYSTEM` first. If you ever see that message, the step did nothing, whatever the exit code says.
- The app's `version` comes from `git describe` when CMake configures, not on every build. A stale `version` with a changed `elf_sha256` is still a new build. Compare `elf_sha256`, not `version`.
- With the Waveshare factory demo on the board, a `capture` (or Doctor) that keeps the port open while the demo boots leaves the USB reset lines dead. esptool then fails with `No serial data received` until someone unplugs and replugs the board. TouchDeck firmware doesn't do this; it was checked with Doctor followed by esptool on 2026-10-24. If the board runs anything other than `touchdeck`, read its identity with esptool only and don't run `capture`.
- If esptool ever says `No serial data received` or `Write timeout` on TouchDeck firmware, don't retry in a loop. Ask the user to replug the board, or to hold BOOT, tap RESET, and release BOOT for download mode. Record that in the report.

For firmware updates, rollback, OTA-aware Doctor, crash builds, and `$TD idf build-newer` (a test-only bundle one patch above the Device for Companion update steps), read [features/update.md](features/update.md).
