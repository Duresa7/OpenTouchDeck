# Firmware reference

This reference is for people who build or change the deck firmware.
To use Open TouchDeck, read the [user guide](usage.md) instead.

## Build and flash

This section builds and flashes firmware from source. To use a matching `.tdfw`
bundle through the app instead, follow [Install or update deck firmware](usage.md#install-or-update-deck-firmware).
Close the desktop app's connection before a flash tool or serial monitor opens
the same port.

The firmware is an [ESP-IDF](https://docs.espressif.com/projects/esp-idf/)
project in the `firmware/` folder, tested with ESP-IDF v6.1. Install ESP-IDF
with the
[ESP-IDF Installation Manager](https://docs.espressif.com/projects/idf-im-ui/),
then either use the ESP-IDF VS Code extension (open the `firmware/` folder and
click Build, Flash and Monitor) or run these from an ESP-IDF terminal:

```sh
cd firmware
idf.py set-target esp32s3   # first time only
idf.py -p COM3 flash monitor
```

Replace `COM3` with your board's serial port. Press `Ctrl+]` to exit the
monitor. Board defaults (flash size, PSRAM, CPU speed, LVGL options) live in
`firmware/sdkconfig.defaults`. The first build downloads LVGL and the LVGL
adapter and cJSON from the ESP-IDF component registry, so it needs a network connection.
`firmware/dependencies.lock` pins the versions.

## Serial console

The firmware runs a debug console on the board's USB port with the prompt
`td> `. Each command answers with one line, `@td ` followed by a JSON object
that holds `cmd`, `ok`, and the command's own fields:

```
td> version
@td {"cmd":"version","ok":true,"project_name":"touchdeck",...}
```

`help` lists the commands in this order:

| Command | Owner | What it does |
|---|---|---|
| `help` | console | Lists the console commands. |
| `version` | console | Reports the app descriptor. |
| `heap` | console | Reports free heap. |
| `uptime` | console | Reports milliseconds since boot. |
| `reboot` | console | Restarts the board. |
| `screenshot` | display | Streams the screen as RGB565 base64 lines, scale 1, 2, or 4. |
| `perf` | display | Measures frames per second. The reply arrives when it is done. |
| `backlight` | display | Turns the backlight on or off. With no argument it reports it. |
| `ui` | ui | Reports the deck and drives its pages and tiles. |
| `layout` | ui | Reports the loaded layout, reloads it from storage, or switches to the built-in one. |
| `touch` | touch | Injects taps and swipes, logs real touches, and reports touch statistics. |
| `keys` | keys | Drives the key engine, reads its state, logs and records its reports, and fuzzes the reducer. |
| `power` | power | Reports the idle screen-off state, or sets the idle timeout until the next reset. |
| `link` | link | Reports the app link state. |
| `store` | storage | Inspects, reads, and writes the files on the storage partition. |
| `settings` | storage | Reads or changes the settings that persist across resets. |
| `@tds` | storage | Carries one base64 line of a `store put`. The host sends it after the put. |
| `recovery` | storage | Reports the recovery hold watcher, or arms recovery and restarts. |
| `update` | update | Reports the firmware slots and update state. |

An unknown command answers with `"ok":false` and an `error`.

A component adds commands with `td_console_register`, which takes an array of
`esp_console_cmd_t`. `app_main` calls it for every component before
`td_console_start`, which seals the list. A registration after that fails with
`ESP_ERR_INVALID_STATE`.

Replies leave through the USB driver and wait for the host when its buffer is
full, so a redrawing display cannot drop them. The prompt and typed echo use
the same writer. Log lines still go through stdio and can be dropped.

Input waits in a 2048-byte queue while a command runs. The queue accepts 2047
bytes, which holds about 250 short lines or seven full-length ones. When a burst
fills it, the firmware drops the next read of up to 64 bytes and then everything
through the next CR or LF, so a line that a drop cut never runs. It reports the
loss before the next prompt:

```
@td {"cmd":"console","ok":false,"error":"input overflow","dropped_bytes":N}
```

`dropped_bytes` counts every byte the firmware threw away, the cut line
included. It counts the rest of a cut line as those bytes arrive, which can be
after the first notice printed, so one loss can print several of these lines.
Add up every one.

The firmware cannot count a loss inside the USB driver. The driver's interrupt
copies each USB packet into a 256-byte ring and ignores the result of
`xRingbufferSendFromISR`. If `td_rx`, the task that empties the ring, cannot
run while about four full packets arrive, the next packet is lost with no
count.

When no line is blank and every line ends in one CR or one LF, the bytes of the
lines that got a reply, line endings included, plus the `dropped_bytes` of every
notice equal the bytes you sent. CR LF breaks that, by up to one byte each time
the queue overflows, in either direction:

- The dropped input can end between a line's CR and its LF. The console reads
  that LF as an empty line, which nothing counts, so the sum comes up one byte
  short.
- The dropped input can start on the LF of a line that already got a reply. That
  LF is counted twice, so the sum runs one byte over.

To check the sum exactly, end every line with LF. A script that sends a burst
should wait for replies after about 250 short lines.

The console reads the USB-Serial/JTAG driver, so it needs USB-Serial/JTAG as
the primary console, which `firmware/sdkconfig.defaults` sets. With another
primary console the firmware still boots, logs that the console is off, and
does not start it.

## App link

The [framed app link](../schema/protocol/link-v1.md) shares the debug console's
USB-Serial/JTAG port. SLIP frames carry CRC-protected payloads of up to 1024
bytes. The `hello`, `ping`, and `link.state` messages identify the running build,
echo a request, and report the connection state. The text command `link state`
reports the same counters. The port stays available for the debug console and
ROM flashing.

The link sends its frames through a carrier, declared in
`firmware/components/td_link/include/td_link_carrier.h`. USB, SLIP over the
USB-Serial/JTAG port, is added first and is the only carrier in the normal
build. The radio build adds net-v2's TCP carrier. Each carrier keeps its own
online state: a `hello` on it brings it online, and 5 seconds with no frame on it,
or the end of its connection, take it offline. A reply or an ACK leaves on the
carrier its request arrived on. Events and the Device's chunks leave through one
active carrier. That is the first carrier that is online, so USB wins whenever the
Companion talks over it. While no carrier is online, it is the carrier the last
frame arrived on. Frames still arrive through the console's frame handler, with
the carrier as its context, and `link state` reports the active carrier's
`online`, `last_rx_ms` and name in `carrier`. A network carrier gets only the
messages net-v2 allows; Firmware updates and every other USB-only message answer
`reason:"usb-only"` there.

The link also moves files and mirrors input. `layout.put` and `layout.get` copy a
layout in chunks, `icon.put`, `icon.list`, and `icon.rm` manage the icons, and
`settings.get`, `settings.put`, and `store.stat` read and change the deck. A put
is checked and validated before it replaces the stored file, and a layout for the
orientation on screen is shown without a reboot. While the app is connected, the
deck sends a `press` event for each app or widget tap and a `key.report` event
for each key report the engine emits, so the app can type the keys on the PC.
`widget.set` fills a widget tile with a value, a caption, and a bar, and
`dashboard.set` fills a dashboard page with up to 8 rows. The deck draws them on
its next 100 ms redraw.

The local verification harness provides `link hello`, `link ping --count 100`,
`link state`, `link raw --hex <frame>`, and `link selftest`. `link scenario <file>`
sends timed console lines and binary frames through one port handle. Console
and link tasks reply independently, so callers match each channel separately.
`link selftest` checks the [golden frames](../schema/protocol/golden-frames.json)
without a board. Live checks still require the board.

## Storage and settings

The `storage` partition holds one [LittleFS](https://github.com/joltwallet/esp_littlefs)
volume, mounted at `/storage` before the console starts. The first boot after the
partition was erased formats it and logs `store: formatted`, and every boot logs
`store: mounted, <used>/<total> bytes`. Flashing the firmware leaves the volume
alone. The paths the firmware uses are `/storage/settings.json`,
`/storage/layouts/<orientation>.json`, `/storage/icons/<id>.bin`,
`/storage/assets/<digit>/<sha256>.bin`, and `/storage/recovery`.

A write goes to `<path>.tmp`, is flushed with `fsync`, and is renamed over the
target. LittleFS makes the rename atomic across power loss, so a reader sees the
whole old file or the whole new one. A temporary file that a power cut left behind
is removed at the next mount.

Settings are `{"idle_timeout_s": 600, "orientation": "landscape"}`. The deck reads
them before the display starts and logs `settings: idle_timeout_s=<n>
orientation=<o>`. An unknown field is ignored, a missing field takes its default,
a timeout outside 1 to 86400 takes 600, and an unknown orientation takes
`landscape`. The idle timeout reaches the power gate at boot. The orientation
applies at boot. In `portrait` the screen is the panel turned 90 degrees
clockwise, 600 by 1024, and the deck loads `/storage/layouts/portrait.json`, or
the built-in layout when that file is missing. Touch points, `touch` and `ui hold`
injection, and `screenshot` all use the rotated size. The adapter rotates each
frame in software, so portrait draws much slower than landscape.

The `store` and `settings` console commands work on the volume:

- `store stat [path]` reports the volume (`total`, `used`, `files`, `dirs`), a
  directory's counts, or a file's size.
- `store ls <dir>` lists a directory, `store rm <path>` removes a file or an empty
  directory.
- `store put <path> <bytes> <crc32>` takes the file from the host as `@tds ` base64
  lines of 180 raw bytes, up to 65536 bytes. The deck holds the file in memory and
  swaps it in atomically after its last line. It acknowledges every 6 lines, so the
  host never sends more than one window ahead.
- `store get <path>` streams a file as `@tds ` lines between a `start` and an `end`
  reply that carries its CRC-32.
- `settings get` and `settings set idle_timeout_s <n>` or
  `settings set orientation landscape|portrait` read and write the file. A new
  timeout applies at once. A new orientation applies on the next boot, so its
  reply, like `settings.put`'s, carries `reboot_required` true.

The local verification harness wraps the transfers as `serial store put <path>
--from <file>` and `serial store get <path> --out <file>`, checking the CRC-32
both ways.

## Recovery

Holding one finger on the glass for 10 seconds restarts the deck into a known-good
state, whatever the deck shows and whether or not the backlight is on. The hold
arms when one contact stays within 24 px of where it began for 10000 ms. A
release before that, or a move past 24 px, starts over. The recovery watcher reads
the raw contact before the idle gate, so a hold that begins on a dark screen
counts from its first sample, and that same contact wakes the panel.

When the hold arms, the deck stops sending the contact to the pages, so no tile
fires when the finger lifts. It draws a full-screen `Release to reset` band, logs
`recovery: armed`, writes the marker file `/storage/recovery`, and restarts 250 ms
after the lift.

A boot that finds the marker deletes it and runs in recovery mode. The deck shows
the built-in pages and the default settings from RAM, leaves every file in
`/storage` as it was, logs `recovery: mode on, files kept`, and shows a `Recovery`
caption in the title band. The link's `hello` and `layout state` report
`"recovery": true`, and `layout state` still names what is wrong with a stored
layout that does not parse. A layout or settings write by the app, a
`layout use storage`, or a restart leaves recovery mode. A plain `layout reload`
in recovery mode reloads the built-in pages.

The `recovery` console command has two forms:

- `recovery state` replies with `mode`, `watcher` (`IDLE`, `HOLDING` or `ARMED`),
  `held_ms`, and `armed_count`.
- `recovery arm` writes the marker and restarts, as the hold does after the lift. It
  is for tests.

A swipe lasts at most 2000 ms, so `touch begin <x> <y>`, `touch move <x> <y>` and
`touch end` hold an injected contact for as long as a test needs and return at
once. A contact that nobody ends lifts by itself after 60 seconds. `touch stats`
adds `watch_samples`, `watch_ns_avg`, and `watch_ns_max`, which is what the watcher
costs per sample.

## Firmware updates

The app updates the deck's firmware over the link, and the deck rolls back by
itself when the new image does not come up. The flash holds two application slots,
`ota_0` and `ota_1`, of 4 MB each. An update writes the slot the deck is not
running, so the running image stays whole until the very end.

`update.begin {bytes, sha256}` opens the other slot and `update.end` finishes it, with
the image carried by the same chunk transfer as a layout. At `update.end` the deck
checks the SHA-256 of the bytes it wrote, validates the image, sets the new slot as
the boot slot, replies with its name, and restarts 250 ms later. A refusal replies
with `reason` and leaves the boot slot alone: `size`, `sha256`, `incomplete`,
`flash`, `busy`, `unconfirmed`, or `project`. A transfer that stops moving for 2
seconds is dropped, and the update returns to `IDLE`.

The new image boots as `PENDING_VERIFY`. It confirms itself after the first frame,
a layout load, and 10 seconds of uptime, and logs `update: confirmed <slot>`. A
reset before that, from a crash, a watchdog, or a power cut, makes the bootloader
mark the slot `ABORTED` and boot the previous one. While an image waits to confirm,
the deck refuses another update.

The `update` console command has three forms:

- `update state` replies with `running`, `boot`, the state of `ota_0` and `ota_1`
  (`NEW`, `PENDING_VERIFY`, `VALID`, `INVALID`, `ABORTED`, `UNDEFINED`, or `NONE`
  when the slot has no record), `confirmed`, and the update `state`.
- `update confirm` confirms the running image at once. It is for tests.
- `update invalidate` marks the running image invalid and reboots into the other slot,
  when that slot holds a valid image. It is for tests.

`firmware/tools/make_bundle.py` packs a build into `touchdeck-<version>.tdfw`, a zip
with `bootloader.bin`, `partition-table.bin`, `touchdeck.bin`, and a `manifest.json`
that names each file's flash offset, size, and SHA-256 with the image's version and
`elf_sha256`. `python firmware/tools/make_bundle.py --check <bundle>` reads one back.

A flash over the cable writes `ota_0` and resets the boot slot to it, so
`idf.py flash` always recovers the deck. The local verification harness provides
`link update <touchdeck.bin> --wait` and `idf build-crashing`, a build that aborts
two seconds after start, for checking the rollback on the board.

## Display

On boot the panel shows the deck, drawn with LVGL 9: dark rounded tiles on a
near-black background, with page dots at the bottom. It has two pages,
`F-Keys` and `Media`, and a swipe left or right changes the page. A swipe past
either end bounces back. Tiles on `F-Keys` send F1 to F12, and Shift, Ctrl, Alt,
and Win arm a modifier for the next key. Tiles on `Media` send previous, play or
pause, next, mute, volume down, and volume up. The reports go through the key
engine to the console key log and the USB app link. The connected desktop app
mirrors them into the host session. Built-in pages come from embedded JSON in
[builtin.json](../schema/examples/builtin.json), and saved layouts use the same
[layout format](../schema/README.md). The Backlit look is flat: every fill is one
RGB565 color, with no shadow, glow, border, or gradient. The screen colors are
macros in `firmware/components/ui/ui_theme.h`, one per token of
`app/ui/src/styles/tokens.css`. The sizes, radii, and animation times are named
fields of the theme in `firmware/components/ui/ui_theme.c`.

The `ui` component adds the `ui` console command: `ui state` reports the page and
the tile that is pressed, `ui goto <page>` changes the page, `ui tile <id>` gives a
tile's box, and `ui hold <x> <y> <ms>` holds a press on the glass for a test.
`ui state` also reports `online`, whether the app is connected, and `dimmed`, how
many tiles are drawn dim. `ui widget <tile>` and `ui dashboard <page>` report what
the app last sent to a widget tile or a dashboard page.

The display component adds three console commands:

- `screenshot [1|2|4]` streams the screen as RGB565 in `@tdb ` base64 lines and
  ends with a reply that carries a CRC-32. Scale 2 or 4 keeps every second or
  fourth pixel.
- `perf fps <seconds>` redraws the whole screen every frame and replies with the
  frames per second when the time is up.
- `backlight on|off` switches the backlight through the CH422G expander.

The display component hands its commands to `td_console_register`, so `help`
lists them. Until the display has started, `screenshot` and `perf` answer
`display not ready`.

### Fonts

The deck draws its text in six Archivo fonts, generated as LVGL C files into
`firmware/components/ui/fonts/`, with the font's
[SIL Open Font License](../firmware/components/ui/fonts/OFL.txt) next to them.
The commands below give each one's instance, size and glyph set. The `lv:`
icons are LVGL symbols, which only LVGL's own Montserrat 48 carries, so they
still draw in it. The bar's speaker and the now-playing controls are the same
symbols at the 32 px glyph size, in Montserrat 32.

The source is `Archivo[wdth,wght].ttf`, version 2.001, from the upstream
repository at commit `b5d63988ce19d044d3e10362de730af00526b672`:
<https://github.com/Omnibus-Type/Archivo/blob/b5d63988ce19d044d3e10362de730af00526b672/fonts/variable/Archivo%5Bwdth,wght%5D.ttf>.
`OFL.txt` comes from the same commit.

To regenerate them, install fontTools 4.60.1 (`pip install fonttools==4.60.1`)
and lv_font_conv 1.5.3 (`npm install lv_font_conv@1.5.3`). Then cut the three
instances and convert each font in a folder of its own, and copy the six `.c`
files into `firmware/components/ui/fonts/`:

```sh
fonttools varLib.instancer "Archivo[wdth,wght].ttf" wght=520 wdth=100 -o Archivo-Medium.ttf
fonttools varLib.instancer "Archivo[wdth,wght].ttf" wght=620 wdth=100 -o Archivo-Semibold.ttf
fonttools varLib.instancer "Archivo[wdth,wght].ttf" wght=620 wdth=118 -o Archivo-Nameplate.ttf

R="-r 0x20-0x7E -r 0xA0-0xFF -r 0x2013-0x2014 -r 0x2018-0x2019 -r 0x201C-0x201D -r 0x2022 -r 0x2026 -r 0x2212"
conv() { lv_font_conv --font "$1" --size "$2" --bpp 4 --format lvgl --no-compress --lv-include lvgl.h $3 -o "$4.c"; }
conv Archivo-Medium.ttf 22 "$R" ui_font_medium_22
conv Archivo-Medium.ttf 26 "$R" ui_font_medium_26
conv Archivo-Medium.ttf 44 "-r 0x20-0x7E" ui_font_medium_44
conv Archivo-Semibold.ttf 32 "$R" ui_font_semibold_32
conv Archivo-Semibold.ttf 40 "$R" ui_font_semibold_40
conv Archivo-Nameplate.ttf 26 "$R" ui_font_nameplate_26
```

`--no-compress` stores the glyphs as plain 4-bit bitmaps, as LVGL's built-in
fonts do, so drawing a glyph reads it straight from flash with no
decompression. `--lv-include lvgl.h` makes the files include `lvgl.h` the way the
ESP-IDF LVGL component provides it. The same tool versions and commands give
the same files byte for byte.

The app's Device Previews measure and break text with the same fonts. After
you regenerate them, run
`node app/ui/src/lib/deck/fonts.extract.mjs firmware/managed_components/lvgl__lvgl/src/font`
from the repository root to rewrite `app/ui/src/lib/deck/fonts.data.ts`. The
UI tests fail when a Preview font's size or line height no longer matches the
Firmware font `ui_theme.c` names for it.

### Performance options

Five Kconfig options make the firmware faster or lighter without changing what
the screen shows. Each is on by default, and menuconfig can turn it off:

- `CONFIG_TD_PAY_COPY_DEBT`, in `TouchDeck display`. In the adapter's
  triple-partial mode a refresh that redraws most of the screen, such as a page
  slide's frame, leaves the two other frame buffers behind, and each of the next
  two refreshes copies most of the screen into one of them, about 60 ms each.
  With this option LVGL redraws one unchanged pixel twice once it has been idle
  for 120 ms, so those copies happen before the next touch. A tap half a
  second after a slide then finishes its frame in about 7 to 24 ms instead of
  62 to 66 ms.
- `CONFIG_TD_LV_TICK_CB`, in `TouchDeck display`. LVGL reads its millisecond
  clock from `esp_timer_get_time()`, so the adapter's 1 ms tick timer no longer
  wakes the `esp_timer` task a thousand times a second. Core 0 is about a third
  less busy on a static screen.
- `CONFIG_TD_LAZY_SNAPSHOT_BUF`, in `TouchDeck display`. The 1.2 MB PSRAM buffer
  that `screenshot` renders into exists only while a screenshot or
  `perf fb check` runs.
- `CONFIG_TD_LAZY_KEY_RECORD`, in `TouchDeck keys`. The 40 KB key record is
  allocated by the first `keys record start` instead of at boot. Before that,
  `keys record dump` answers with a count of 0.
- `CONFIG_TD_UI_SKIP_NOOP_SETS`, in `TouchDeck UI`. A widget or dashboard update
  sets a label's text or a widget's text opacity only when the label would
  change, so an update that repeats the shown values draws no frame. A label
  cut with dots is always set again, so a repeated value too long for its tile
  or row still redraws.

### Perf tools (test builds only)

`CONFIG_TD_PERF_TOOLS`, in the menu `TouchDeck perf tools (test builds only)`,
adds measurement commands for performance work. It stays off in `sdkconfig` and
`sdkconfig.defaults`, and while it is off the firmware compiles to the same code
as without it. A perf build goes to a directory of its own with a config of its
own, on top of the shipped defaults. From `firmware/`:

```sh
idf.py -B <abs>/build-perf -D SDKCONFIG=<abs>/build-perf/sdkconfig \
       -D "SDKCONFIG_DEFAULTS=sdkconfig.defaults;perf/tools.defaults" build
```

Flash it with the same `-B` and `-D` options and `-p <port> flash`.
[tools.defaults](../firmware/perf/tools.defaults) turns the tools on, with
FreeRTOS run-time stats on `esp_timer`. A build that also changes other options
lists its own defaults file after it and names itself in
`CONFIG_TD_PERF_VARIANT`. A perf build never formats a storage volume that will
not mount. It logs an error and boots without storage, so the layout and
settings on the deck stay as they are. `perf` then also takes:

- `perf info` reports the build: PSRAM, flash, and CPU clocks, the panel clock,
  bounce buffer lines, frame buffers, tear mode, rotation, LVGL draw units and
  assembly, refresh period, cache sizes, the variant name, which performance
  options are on (`lv_tick_cb`, `lazy_snapshot_buf`, `lazy_key_record`,
  `pay_copy_debt`), `reset_reason`, and `boot_count`, the boots since power-on.
  With `CONFIG_TD_PAY_COPY_DEBT` on, `copy_debt_steps` counts the idle redraws
  it has asked for since boot.
- `perf fps <seconds>` keeps its fields and adds `shown`, `shown_fps`,
  `refreshes`, `render_us_avg`, `render_us_max`, `render_only_us_avg`,
  `render_only_us_max`, `copy_us_avg`, `copy_us_max`, `flush_us_avg`,
  `flush_us_max`, `px`, `cpu0_pct`, and `cpu1_pct`.
- `perf window start` and `perf window stop` report the same over any stretch,
  with `elapsed_ms` and `drawn`, and force no redraw.
- `perf tap <x> <y>` taps like `touch tap` and replies with `press_to_flush_us`,
  `press_to_shown_us`, `release_to_flush_us`, and `release_to_shown_us`, each null
  when no redraw followed within 500 ms. `press_render_us`, `press_copy_us`,
  `press_strips`, and `press_px`, and the same for `release`, describe the
  refresh that flushed. The tap's press and release samples usually reach LVGL
  in one input read, about 1.4 ms apart, so both name the same refresh. In the
  adapter's triple-partial mode the first two partial refreshes after a
  full-screen redraw, such as boot or `perf fb prep`, each copy the whole screen
  into a lagging back buffer, about 60 ms, which `press_copy_us` shows, unless
  `CONFIG_TD_PAY_COPY_DEBT` paid it while idle. `shown_chained` is true, because
  the hook that takes `shown` calls the adapter's own frame-complete callback
  after it.
- `perf mem` reports free, minimum free, and largest block of internal, PSRAM, and
  DMA heap, and each task's core, priority, stack size, and stack high-water mark.
  Minimum free sizes count from the start of `app_main`, because ESP-IDF gives
  the 32 KB internal pool it reserves after startup
  (`CONFIG_SPIRAM_MALLOC_RESERVE_INTERNAL`) a minimum of 0, which would put the
  internal minimum 32 KB below anything in use. `perf mem boot` lists free,
  minimum free, and largest free internal RAM and free PSRAM after each boot
  stage. `perf mem heaps` prints ESP-IDF's per-heap summary of internal RAM to
  the log and replies with the totals. `perf mem last` reports what the previous
  boot saved when it restarted: its latest sample, taken every two seconds, of
  each heap's free and minimum free size and each task's stack high-water mark,
  with `boot`, the boot that saved it, and `uptime_ms`, the sample's time. When
  the previous boot saved nothing it replies `"saved":false`, with `stale_boot`
  when an older boot's record is left. A panic, a watchdog, or a power cut saves
  nothing.
- `perf cpu <seconds>` replies when the time is up with each core's busy share and
  each task's share of one core.
- The task lists of `perf mem`, `perf mem last` and `perf cpu` read each name
  from the running system without stopping the other core. A task that ends at
  that moment is left out or shows a garbled name. The read stops at the name's
  length limit, and the figures beside it are unaffected.
- `perf fb prep` redraws the whole screen three times, so all three panel frame
  buffers hold the same frame. `perf fb check` redraws nothing. It takes a
  snapshot, copies each panel buffer to PSRAM with the LVGL lock held, and
  compares it with the snapshot, turned the way the adapter turns it in portrait.
  Per buffer it replies with `index`, `crc`, `exact`, `max_diff` (the largest
  RGB565 channel step), and `diff_pixels`, plus `submits`, the frames handed to
  the panel since the last prep, and `screen changed` when a frame flushed in
  between. Each copy holds the lock for about 60 ms.
- `perf invalidate <x> <y> <w> <h>` marks one area of the screen for redraw.
- `perf stall <ms>` suspends the LVGL task for up to 10 s and replies at once,
  so a `screenshot` sent next runs into its timeout.
- `perf memtest <kb>` writes six patterns, among them each word's own address,
  over a PSRAM block of its own, reads them back from PSRAM, and replies with
  `errors`, `bytes`, and `us`. It touches no other memory. `kb` goes up to 4096
  and to the largest free PSRAM block that `perf mem` reports.
- `perf membw` replies with memcpy rates in MB/s, SRAM to PSRAM, PSRAM to SRAM,
  and PSRAM to PSRAM, measured on core 1 with the panel running. Its 4 KB SRAM
  buffer shows in the internal minimum free size afterward.
- `perf temp` replies with the chip temperature in degrees Celsius as `temp_c`.
  The first reading turns the sensor on, and it stays on until the next reset.

`drawn` counts LVGL refreshes that flushed, `shown` the panel refreshes that
scanned out a new frame, and `refreshes` every panel scan-out, about 24 a second.
`render` runs from the start of an LVGL refresh to its last flush call, and
`flush` from that call to flush ready. LVGL draws a refresh in strips and the
adapter copies each strip into the frame buffer before the next is drawn, so
`render` includes those copies. `render_only` leaves them out, and `copy` adds up
every strip's flush, the last one's included. `px` counts the pixels flushed. A
core's busy share is 100 % less its idle task's share of run time, and interrupt
time counts toward the task it interrupted.
`firmware/components/display/display_perf.c` says how each figure is taken.

## Radio build

The radio build is an opt-in second Firmware configuration with Wi-Fi, lwIP
and NimBLE. The installer's Firmware is the normal build, and it stays
radio-free. A release can also ship the radio build as a separate file,
`touchdeck-radio-<version>.tdfw`, with the Companion carrying both; then Device
settings in the app switches a Device between them, both ways. Whether it does
is set in `app/release-options.json` (see
[packaging](packaging.md#the-radio-build-in-a-release)). An update keeps the
build the Device runs. Build the radio build into a directory of its own.
From `firmware/`:

```sh
idf.py -B <abs>/build-radio -DTD_RADIO_BUILD=1 build
```

`-DTD_RADIO_BUILD=1` adds the `td_radio` component and layers
[radio/radio.defaults](../firmware/radio/radio.defaults) over
`sdkconfig.defaults`. The build keeps its `sdkconfig` in the build directory,
so the committed `sdkconfig` stays the normal one. CMake saves the option in
the build directory, so every later build there is a radio build too, even
without `-D`. Keep the radio build in its own directory. A radio build in the
default `firmware/build`, the folder the normal Firmware is flashed from, stops
with an error. Flash it with the same `-B` and `-D` options and
`-p <port> flash`. For the perf tools as well, add
`-D "SDKCONFIG_DEFAULTS=sdkconfig.defaults;perf/tools.defaults"`; the radio
options still go on top.

What the radio build does:

- Wi-Fi and lwIP buffers come from PSRAM where ESP-IDF allows it
  (`CONFIG_SPIRAM_TRY_ALLOCATE_WIFI_LWIP`), and the internal buffers are sized
  small. The Wi-Fi driver stores nothing in NVS; the station keeps its one
  network itself (below).
- Static `.bss` goes to PSRAM where ESP-IDF allows it
  (`CONFIG_SPIRAM_ALLOW_BSS_SEG_EXTERNAL_MEMORY`): the Wi-Fi driver's, lwIP's
  and wpa_supplicant's, and the link's four frame buffers (`EXT_RAM_BSS_ATTR`
  in `td_link.c`, which is empty in the normal build). That frees about
  18.6 KB of static internal RAM, more than the 4.3 KB that linking the BLE
  controller and NimBLE costs.
- NVS stays unencrypted, and no build burns an eFuse or enables flash
  encryption. Both NVS encryption schemes on the ESP32-S3 are irreversible, so
  the radio build waits for an operator decision
  ([ADR 0010](adr/0010-nvs-stays-unencrypted-until-the-operator-decides.md)).
  Anyone who dumps the Device's flash can read its Pairings and impersonate the
  Device, because the long-term key is symmetric
  ([net-v2](../schema/protocol/net-v2.md#the-long-term-key-is-symmetric)).
  `radio_guard.py` fails either build when `NVS_ENCRYPTION`, flash encryption,
  secure boot or anti-rollback is on.
- Internal RAM is tuned to the floor. `malloc` puts blocks of 256 bytes and up
  in PSRAM first, not 16 KB and up. The heartbeat runs on an `esp_timer`, so
  `app_main` returns its stack. The lwIP, event, `esp_timer` and FreeRTOS timer
  task stacks are sized from the board's high-water marks. The event task gets
  768 bytes more, because the station logic runs on it.
- LVGL's objects, styles and strings go to PSRAM first
  (`CONFIG_LV_USE_CUSTOM_MALLOC` with `display_lv_mem.c`), and so does the
  console's 2 KB receive queue. Under the 256-byte rule nearly every LVGL
  object stayed internal. The normal build keeps both where they were. Without
  them the Wi-Fi station's start, which takes about 19 KB of internal RAM at
  once, left the main internal heap all but empty at boot (#313).
- After the display and touch start, the Wi-Fi station starts and joins the
  network set over USB, if there is one. It joins again after every restart
  and every drop, after 1, 2, 4 and 8 seconds and then every 30 seconds. It
  never scans on its own: nothing calls `esp_wifi_scan_start`. The network
  comes from the app (Device settings, Wi-Fi) or the console, over USB only,
  with the `wifi.*` link messages in
  [link-v2](../schema/protocol/link-v2.md#wi-fi-network). It lives in NVS, in
  namespace `td_wifi`, so a Layout reset or a Firmware update keeps it. NVS
  stays unencrypted (above), so anyone with the board and a flash reader can
  read the password, as with any ESP-IDF station. Log lines never name the
  network or the password; a failed join logs only its reason, such as
  `td_wifi: join failed: not-found, retry in 1000 ms`. ESP-IDF's own `wifi`,
  `wpa` and `esp_netif_handlers` lines name the network, a nearby access point
  or the address, so the radio build sets those tags to errors only before the
  station starts, and `radio_guard.py` requires `td_wifi_quiet_logs` in the
  image.
- A small Wi-Fi glyph at the right end of the title band shows the station
  while a network is stored: grey when joined, dim while joining, amber after
  a failure. With no network stored it is hidden.
- NimBLE is compiled in, with the controller in flash and NimBLE's memory in
  PSRAM. BLE is off until the person turns on the Bluetooth keyboard (see
  [Bluetooth keyboard](#bluetooth-keyboard-radio-build)). A fresh Device never
  starts the controller or the host, so it never advertises. No observer or
  central role is compiled in, so the Device can never scan or start a
  connection.
- The net-v2 service (`td_netlink`, with the protocol in `td_netlink_core`)
  listens on TCP port 47820 and adds the `wifi` carrier to the link. It pairs
  and opens sessions as [net-v2](../schema/protocol/net-v2.md) describes, with
  its crypto buffers, records and task stack in PSRAM. Only a USB request opens
  a pairing window: the `net pair` console command or the `net.pair` link
  message. The Device then shows the six-digit code, the time left and Cancel.
  The Pairings (up to four), the failed-code count and the lockout live in the
  `td_netlink` NVS namespace. While USB is online, a network session can't
  open, and an open one closes. mDNS discovery is not built yet;
  `td_netlink_core` already builds its private TXT record (nonce and hints,
  plus the id, name and count only in pairing mode) and finds a session's
  Pairing by its `open_tag`, as net-v2 says.
- While the owner is unproven, up to three more connections wait for their
  first item, since a `session.open` carrying a stored app's tag takes over. With every
  slot taken, a new connection pushes out the oldest waiter that hasn't sent a
  byte, or else the oldest from a peer in backoff, so idle sockets can't keep a
  paired app out. A waiter that misses the 2-second first-item deadline or hangs
  up first counts toward its address's backoff. Refusals and closes write their
  last item and drain from the net task's poll loop, never blocking it.
- Each carrier counts its ended connections. A request that waited in the link
  while its session ended is not handled, a reply or event for an ended session
  is dropped, and a transfer moves only on the connection that opened it: other
  carriers' chunks are dropped, and the transfer is cut when its session ends.
- 30 seconds after the station starts, the log reports the steady-state
  internal heap against the 24 KB floor (`CONFIG_TD_RADIO_INTERNAL_FLOOR`):
  `radio: steady free_internal=N largest_internal=N floor=24576`. Below the
  floor, it logs that line as an error with `below floor`. When the station
  did not start, the floor is not measured: the line is an error, and
  `floor_ok` stays false. The next line reports the lowest free internal heap
  since startup, which the station's start sets at boot, against the 16 KB
  boot floor (`CONFIG_TD_RADIO_BOOT_FLOOR`):
  `radio: boot min_free_internal=N floor=16384`, an error with `below floor`
  under it. The figure is ESP-IDF's minimum free size, the same as `heap`'s
  `min_free_internal`. ESP-IDF gives the 32 KB internal pool it reserves after
  startup (`CONFIG_SPIRAM_MALLOC_RESERVE_INTERNAL`) a minimum of 0, so the
  figure leaves that pool out, and the real headroom is that much larger.
- The console gets three more commands, after `update`: `ble` (see
  [Bluetooth keyboard](#bluetooth-keyboard-radio-build)), `wifi` (below) and
  `radio state`, which replies with
  `build`, `wifi` (`started`, `failed` with `wifi_error`, or `off`), `joined`,
  `ble` (`off`, `starting`, `on` or `failed`), `free_internal`,
  `largest_internal`, `floor`, `floor_ok`, `min_free_internal`, `boot_floor`,
  `boot_floor_ok` (false while the station is not started), `steady`, and,
  after the steady-state report, `steady_free_internal` and
  `steady_largest_internal`. `heap` and, in a perf build, `perf mem` work as in
  the normal build. `wifi state`, `wifi set <ssid> <password>` and
  `wifi forget` do what the `wifi.*` link messages do and reply with the same
  fields. Quote a name or password that holds spaces, as for any console
  command: `wifi set "my network" "my pass phrase"`. The console echoes `*` for
  every byte of the password as it is typed, so a screen, a recording or the
  serial log never shows it, and the command still gets the real text.
- In the normal build, every `wifi.*` link message gets the unknown-message
  error, the hello has no `wifi` capability, and there is no `wifi` command.
- `net state` replies with net-v2's `net.state` fields: `listening`, `port`,
  `ble`, `session`, `paired`, `pairing`, `pair_failures` and `locked_ms`.
  `net pair` opens a pairing window, and `net forget all` erases every Pairing
  and ends the session. They fail with `reason` `locked`, `full` (four
  Pairings: no window opens) or `storage`. The link has the same three as
  `net.state`, `net.pair` and `net.forget`; the last two are USB-only.
  `net.forget` needs `all:true` and replies with `removed`; a bare one, or one
  with `app_id`, fails with `reason:"args"`. `net.list` and forgetting one app
  aren't built yet.
- A session ends between requests once fewer than 1024 of its 2^24 records
  remain in either direction, and the Companion reconnects for fresh keys.

`firmware/tools/radio_guard.py` checks a build directory for its kind:

```sh
python firmware/tools/radio_guard.py normal <build dir>
python firmware/tools/radio_guard.py radio <build dir>
```

- `normal` fails on any Wi-Fi, Bluetooth or `TD_RADIO_BUILD` option, and on
  any radio or network component linked: `esp_wifi`, `esp_phy`, `esp_coex`,
  `wpa_supplicant`, `bt`, `esp_netif`, `lwip`, `td_radio`, `td_net`, `td_ble`,
  and any netlink or mDNS component. The release recipe's audit runs the same
  check.
- `radio` requires the radio options and components, and fails when one of
  those options is missing, the floor is set below 24576 or the boot floor below
  16384. The options include LVGL's PSRAM-first allocator and
  the Bluetooth keyboard's: LE Secure Connections only, privacy (the resolving
  list), bonds in NVS, one connection, four bonds, no extended advertising, and the Peripheral Preferred
  Connection Parameters. With
  `--size-json <file>`, the output of `idf.py size --format json2`, it fails
  when static internal RAM (DIRAM) is over 153,600 bytes. On the board, 151,083
  bytes left 28,815 free in steady state, so raise that ceiling only with a new
  board measurement. With the Bluetooth keyboard and `.bss` in PSRAM, the build
  uses about 136,800 bytes. It also reads the linked
  image with the toolchain's `nm`, and fails when the image contains any
  function that scans or starts a connection over BLE, advertises in extended
  mode, or scans for a Wi-Fi network, or when it lacks `td_ble_boot`,
  `td_netlink_start` or `esp_wifi_connect`, which the station needs to join
  its network. The linker drops every function that nothing calls, so a name
  missing from the image is a call that cannot happen. Its tests also check
  that only `td_radio.c`'s station driver calls `esp_wifi_connect`.

`python -B firmware/tools/test_radio_guard.py` tests both checks, and checks
the Firmware sources for those calls. It also checks that only
`td_ble_nimble.c` calls what starts BLE, advertises or writes the accept list
(`nimble_port_init`, `ble_gap_adv_start`, `ble_gap_wl_set` and the rest of
`BLE_START_SYMBOLS`). With `TD_GUARD_NORMAL_BUILD` or
`TD_GUARD_RADIO_BUILD` set to a build directory, it also checks that real
build. CI builds both configurations on every Firmware change. It uploads the
normal bundle as `firmware-tdfw` and the radio bundle as `firmware-radio-tdfw`,
named `touchdeck-radio-<version>.tdfw`. A radio bundle's `manifest.json` has
`"radio": true`, and the installer recipe refuses such a bundle as
`--firmware-bundle`. It takes one only as `--radio-firmware-bundle`, which
refuses a normal bundle.

The radio build's link-v2 hello carries `radio: true`; the normal build sends
no `radio` field (see [link-v2.md](../schema/protocol/link-v2.md)). The
Companion reports it as the Device's Firmware `channel`.

CI also builds a QEMU test image of the radio build, which runs in Espressif's
QEMU with its emulated Ethernet (`open_eth`) in place of Wi-Fi:

```sh
idf.py -B <abs>/build-qemu -DTD_RADIO_BUILD=1 -DTD_QEMU_TEST=1 build
python firmware/tools/qemu_net_check.py <abs>/build-qemu
```

[radio/qemu.defaults](../firmware/radio/qemu.defaults) sets
`CONFIG_TD_QEMU_TEST`, which runs only the link and the net-v2 service, opens a
pairing window at boot and logs its code. The image has no PSRAM, because QEMU
crashes when the app brings up the board's octal PSRAM, so the net-v2 service
runs from internal RAM there. It also runs flash at 40 MHz, with ESP-IDF's
default cache layout. `qemu_net_check.py` starts QEMU with
port 47820 forwarded to the image, one TCG thread, and the serial port in
`<build>/qemu_serial.log`. Its client,
`firmware/tools/qemu/net_v2_client.mjs`, built on
[net-ref](../schema/protocol/net-ref/), pairs with the logged code, opens a
session, sends `hello`, `link.state` and `net.state` in records, checks that
USB-only messages are refused, and checks that a second connection gets `busy`.
It needs `qemu-system-xtensa` (`idf_tools.py install qemu-xtensa`) and Node 22
or later. The image is never flashed or packed: `radio_guard.py radio` fails any
build with `TD_QEMU_TEST`, `ETH_USE_OPENETH` or their code. It can't check the
internal RAM floors: it has no PSRAM, no panel and no Wi-Fi station, so its
figures say nothing about the board's.

### Radio build board check

A change that can move internal RAM in the radio build (Wi-Fi, lwIP, NimBLE,
LVGL, task stacks, the link or the console) gets a board run with BLE off and
no network set. Flash the radio build, reset the board with the serial monitor
open, and wait at least 35 seconds. The log must show both floors as info
lines, not as errors with `below floor`:

```
I (...) radio: steady free_internal=N largest_internal=N floor=24576
I (...) radio: boot min_free_internal=N floor=16384
```

Then `radio state` must reply with `floor_ok` and `boot_floor_ok` true. On 2026-11-07
the board read a steady `free_internal` of 69,719 and a boot
`min_free_internal` of 30,692 (#313). Before #313 it read 52,963 and 13,364,
and 45,835 and 7,812 before #305 moved the link's frame stream to PSRAM. At
7,812 the station's start had emptied the main internal heap, and only RTC
memory's 7.8 KB and ESP-IDF's reserved pool were left.

## Bluetooth keyboard (radio build)

The radio build can be a Bluetooth LE keyboard with media keys for a phone, a
tablet or another computer: a HID-over-GATT peripheral (`td_ble`) on NimBLE.
It is off by default. The setting persists in NVS (namespace `td_ble`, key
`enabled`), and only a stored `1` starts the controller and the host at boot. A
fresh Device, or one whose setting is off, never starts BLE. The person turns it
on and off, opens the pairing window, lists bonded hosts and forgets them over
USB only: the link's `ble.*` messages
([link-v2.md](../schema/protocol/link-v2.md#bluetooth-keyboard-radio-build))
or the console:

- `ble state`: the setting, the stack (`off`, `starting`, `on`, `failed`), the
  pairing window, whether a passkey is on screen, the advertising (`off`,
  `pairing`, `fast`, `slow`), and the bonded hosts, with the connected ones.
- `ble on` / `ble off`: stores the setting, then starts or stops the stack.
- `ble pair` / `ble close`: opens the pairing window for 120 seconds, or closes
  it.
- `ble forget [aa:bb:cc:dd:ee:ff]`: forgets one bonded host, or all of them.

Tiles with `sink:"bluetooth"` send their keys only to the bonded host that is
connected and subscribed. Omitted sinks send to the Companion. With no host
connected, BLE drops the press without rerouting it.

What a host sees:

- A HID Service with a keyboard report (ID 1: modifiers, a reserved byte, six
  keys), its LED output report (ID 1, one byte), and a consumer report (ID 2:
  one 16-bit usage, 0 to 0x03FF), plus the boot keyboard reports. Every HID
  characteristic (the report map, the reports, Protocol Mode, the Control Point
  and HID Information) needs an encrypted, authenticated link with a bonded
  host to read, write or subscribe: NimBLE checks the `_AUTHEN` flags and the
  access callback checks the bond. Report values carry no Report ID byte. Each
  connection starts in Report Protocol.
- A Battery Service with a fixed level of 100: the board runs from USB and
  can't measure a battery. The level needs the same authenticated link.
- A Device Information Service with the PnP ID: the vendor ID source is the
  Bluetooth SIG, the vendor is Espressif, and the product version is the GATT
  version (`TD_BLE_GATT_VERSION`, now 3). A Device that boots with a new GATT
  version raises Service Changed for its bonded hosts. The PnP ID is the only
  characteristic open to a link that isn't authenticated.

How it behaves on the air, from the crash analysis in #275:

- Pairing happens only inside the window. It uses LE Secure Connections only,
  with MITM protection and bonding, and keeps at most four bonds. The window
  refuses to open with four. While it is open the Device advertises as general
  discoverable at 30-50 ms with its name in the scan response. A new bond closes
  the window. A host outside the window that isn't bonded is dropped.
- The Device is display-only (`BLE_HS_IO_DISPLAY_ONLY`), so a host with a
  keyboard pairs by passkey entry: the Device shows a random six-digit code
  over the Page and the idle face, and the person types it on the host. The
  code comes from the hardware random generator, never leaves the screen (no
  log, no link message), and goes when pairing completes or fails, its link
  drops, the window closes, Bluetooth turns off, or 60 seconds pass, which also
  drops the link. If the screen is dark, a touch lights it without pressing
  anything.
- Passkey entry was chosen over numeric comparison because it keeps the Device
  output-only: confirming a comparison would need a touch path from the UI into
  pairing, where a stray tap or a press still held could confirm. A host that
  can't type (no keyboard, or display and yes/no only) ends up in Just Works,
  and a host that offers only legacy pairing is refused by Secure Connections
  Only mode.
- A link whose keys aren't authenticated Secure Connections keys with a 16-byte
  key (a Just Works or legacy bond, or an old bond made that way) is dropped
  and its bond deleted, whether it just paired or reconnects with stored keys.
  `ble state` counts these in the stats as refused.
- Reconnect advertising is for bonded hosts only (the controller's accept
  list), carries no name, runs at 20-30 ms for 30 seconds and then at 1-2.5 s
  until a host connects. It starts only from an event: the host syncing, a
  disconnect, advertising ending, or a command. Never while the host stack holds
  a connection object, which exists before NimBLE delivers the late `CONNECT`
  event. The accept list is written only after the stack starts and when the
  bonds change. A failed start backs off 1, 2, 4, then 8 seconds.
- Phones and computers reconnect from resolvable private addresses. With
  `CONFIG_BT_NIMBLE_HS_PVCY` the controller resolves them before the accept
  list, using the IRK each host gave when it bonded, so the accept list matches
  the host's identity address. NimBLE loads the stored IRKs when the stack
  starts and adds a new bond's IRK as it stores it. td_ble writes the resolving
  list again from the bond store each time it writes the accept list, removing
  each entry first so none is doubled. A failure there is counted and doesn't
  stop advertising: a host on its identity address still gets in.
- Every event is handled by its own connection handle. A Security Request goes
  out only on a link that isn't encrypted yet. After encryption the Device asks
  once for 7.5-50 ms, latency 0 and a 4-second supervision timeout, the same
  values as its Peripheral Preferred Connection Parameters, and never asks
  again.

The logic lives in `td_ble_core.c`, which has no NimBLE in it, and
`td_ble_nimble.c` is the only file that starts the stack, advertises or writes
the accept list. `python -B firmware/tools/test_ble.py` compiles the core, the
messages and the GATT table with a fake radio and runs two host tests:

- `tools/host/ble_hid_test.c` decodes the report map and checks the Report IDs,
  every Logical Minimum against its Maximum under the signed rule, each
  report's length against what is sent, one Report characteristic per report,
  authenticated encryption on every HID characteristic, the Battery Level and
  every Report Reference (the NimBLE flags themselves, which
  `td_ble_nimble.c` asserts match NimBLE's), the Battery Service, the 512-byte
  limit, and the GATT layout's fingerprint against the version it is pinned to.
  A change to the report map or the table without a new `TD_BLE_GATT_VERSION`
  fails it.
- `tools/host/ble_core_test.c` replays the reboot order (Connection Complete,
  then encryption, then a late `CONNECT`) and a connect that races the end of
  advertising, and checks that nothing retries, the accept list isn't
  rewritten, no Security Request goes out on an encrypted link, and one
  parameter request does. It also checks that BLE stays off unless the stored
  setting is on, the backoff, the fast and slow phases, the pairing window, the
  passkey (an unbiased six-digit draw, on screen until pairing completes, fails,
  times out, its link drops, the window closes or Bluetooth turns off, and
  refused outside the window or for any other pairing method), that Just Works,
  legacy and short-key bonds are refused and deleted, that the resolving list
  follows the bond store before advertising, forgetting, and the `ble.*`
  messages.

It needs a C compiler and one firmware build first, which fetches cJSON, like
`test_link_carrier.py`.

Real radio checks wait for a separate radio (standing order 22: never pair this
PC's adapter with the Device). They are listed in #299.

## Layout format

The versioned [layout format](../schema/README.md) describes pages, tiles, icons,
and actions as JSON. [builtin.json](../schema/examples/builtin.json) preserves the
milestone 1 pages. [dev.json](../schema/examples/dev.json) covers the action types
and a dashboard for the PC app.

Validate the examples or your own layout with Node.js:

```sh
npm ci --prefix schema
node schema/check-layouts.mjs
node schema/check-layouts.mjs path/to/layout.json
node --test schema/
```

Every failure the validator prints starts with a stable rule key, such as
`label.blank`. [schema/README.md](../schema/README.md) lists the keys and the rules
for reading a layout file. Send the deck the compact form of a layout, because
it rejects files over 32768 bytes without parsing them first.

App-side actions and widgets dim on the deck while the app is offline. They
draw at 40 percent opacity, and a tap on one logs `ui: offline <tile>` and sends
nothing. Key, modifier, media, and page tiles never dim.

### Layouts on the deck

The firmware embeds `builtin.json` and draws its pages from it, so the deck has
no page table in C. At boot it loads `/storage/layouts/<orientation>.json`. When
that file is missing or invalid, the deck logs `layout: <reason>, using builtin`
and starts on the built-in layout. Its parser accepts and rejects the same files
as `check-layouts.mjs`, and it reports the same rule key.

- `layout state` reports the layout on screen: `source` (`storage` or `builtin`),
  `name`, `orientation`, `pages`, `tiles`, `icons_loaded`, `icons_missing`,
  `bytes`, `load_ms`, and `error`.
- `layout reload` and `layout use storage` load the stored file again.
  `layout use builtin` switches to the built-in layout. A file that fails
  validation never replaces the layout on screen. `layout state` reports why in
  `error`.
- `ui tile <id>` also reports the tile's `action` type and its `icon`.

A `key` tile with `mods` sends its modifiers together with any active ones. A
`page` tile slides to its page. App and widget tiles send a `press` event over
the app link, and log `ui: press <page>/<tile> <kind>` when no link hook is set.
A `file:<id>` icon is a 64 by 64 bitmap in [schema/icon-format.md](../schema/icon-format.md).

## Unit tests

`firmware/test_app` is an ESP-IDF project that runs the [Unity](https://docs.espressif.com/projects/esp-idf/en/latest/esp32s3/api-reference/system/unity.html)
tests of every component. A component adds tests in its own `test/` folder. List
the test sources in `test/CMakeLists.txt` as `set(TD_TEST_SOURCES test_x.c)`.
Run them on the board from an ESP-IDF terminal:

```sh
cd firmware/test_app
idf.py -p COM3 flash monitor
```

The monitor ends with Unity's summary, `N Tests 0 Failures 0 Ignored`, and `OK`.
Flash `firmware/` again afterwards to get the deck firmware back.

The `td_ui_core` tests also run on the PC. `python -B firmware/tools/test_ui_core.py`
builds `firmware/components/td_ui_core/test` with ESP-IDF's own Unity, the
production Layout parser and cJSON, and runs every test. It needs `IDF_PATH` set
and one firmware build first, which fetches cJSON into
`firmware/managed_components`. It uses `cl` on Windows and `cc` with
`-std=c11 -D_POSIX_C_SOURCE=200809L` elsewhere. CI runs it after the firmware
build, with the other host tests.

`python -B firmware/tools/test_link_carrier.py` compiles the production
`td_link.c` and message dispatcher with host stand-ins for FreeRTOS, the console
and the rest of the Device, from `firmware/tools/host/link/`. It sends link
messages through the USB console's frame handler and through a fake carrier, and
checks which carrier each reply and event leaves on, the USB wire bytes, and the
online state per carrier, that an ended session's queued request and reply go
nowhere, and that a transfer takes chunks only from the connection that opened
it. It also runs the production Device name check against
[name-vectors-v2.json](../schema/protocol/name-vectors-v2.json) and sends each
refused name in `v2.name.set`, which must answer `args`. It needs a C compiler,
`cl` from an x64 Visual Studio developer terminal on Windows or `cc` elsewhere, and one firmware build first,
which fetches cJSON into `firmware/managed_components`. It doesn't need
`IDF_PATH`. It also checks that `wifi.*` messages get the unknown-message
error until the radio build adds them with `td_link_extend`, then `usb-only` on
any other carrier, that the longest hello with the `wifi` and `ble_hid`
capabilities fits one frame, and that after a `wifi.set`, handled, refused or
stale, no link buffer holds its password: not the receive packet, the stream
buffer between the carriers and the link task, the frame body or the message
text.

`python -B firmware/tools/test_wifi.py` runs the radio build's Wi-Fi station
(`firmware/components/td_radio/td_wifi*.c`) over a fake NVS and a fake Wi-Fi
driver, from `firmware/tools/host/wifi/`: credential rules, failure reasons,
retries, the DHCP timeout, a set, a forget and a reboot that rejoins, the
`wifi.*` replies and the `wifi` command. It checks that no reply, console line
or log line holds the password, and that the console's echo mask hides it as
typed. Its compiler and cJSON needs are those of the carrier test.

`python -B firmware/tools/test_netlink.py` compiles `td_netlink_core` and
ESP-IDF's TF-PSA-Crypto for the host and runs every vector and negative in
[net-v2-vectors.json](../schema/protocol/net-v2-vectors.json), then pairing with
the right and the wrong code, the lockout and its persistence, records, the
per-peer backoff, idle connections against a paired Companion, and the USB
rules. It needs `IDF_PATH` and a C compiler, `cl`
from an x64 Visual Studio developer terminal on Windows or `cc` elsewhere.

## Protocol v2 identity

USB discovery retains the v1 hello and advertises protocols 1 and 2. Selecting
protocol 2 adds the factory MAC identity, persistent Device name, Firmware
version, and capabilities. Identity discovery compares the id with the USB
serial after removing colons and hyphens and folding ASCII case; a mismatch
refuses association. The legacy discovery API remains available.

The `td-link` CLI provides `discover-identities`, `identity`, `name`, and
`set-name "Studio"`. Names default to `Open TouchDeck`, contain 1–48 UTF-8
bytes, hold no control or format character or Hangul filler, and persist in NVS separately
from Layouts. See the
[protocol contract](../schema/protocol/link-v2.md) for validation rules.
It advertises the `static`, `graph`, `photo-frame`, and `clock` Tile kinds, the
tdi1 format, and `photo_bytes` 131072. The live values, photo frame, idle, and
clock requests are below.

### Live tiles

A schema 2 Tile with `render: "graph"`, `"bar"` or `"now-playing"` declares the
streams the [protocol contract](../schema/protocol/link-v2.md) lists for it and
draws the values `v2.live.set` sends. The Firmware advertises these caps:

| Cap | Value | Why |
|---|---|---|
| `tile_kinds` | adds `graph`, `bar` and `now-playing` | The Live tiles the Firmware draws |
| `series` | 32 | Samples a graph keeps. 32 doubles is 256 bytes a stream |
| `live_hz` | 2 | Twice the Companion's 1 Hz pace, so jitter isn't refused |
| `live_total_hz` | 16 | 16 requests of 304 wire bytes is about 4.9 KB/s, under the 6,560 bytes/s the link budget gives live values |
| `streams` | 4 | A now-playing Tile has four streams |
| `text_bytes` | 128 | A title or an artist, as the Companion's `fit_text` cuts it |
| `live_images` | 4 | Covers held at once, each up to 128 KiB decoded |
| `formats`, `image`, `asset_bytes` | `tdi1` and `jpeg`, 256 by 256, 32768 | A cover is a JPEG Asset |

Activating a Layout allocates one slot for each declared stream, at most 64,
and one set of objects for each Live tile, in PSRAM: about 300 bytes each, so
under 40 KB. Image streams past the fourth get no slot. Activation clears every
value and `n`. The `layout` a request names is
the SHA-256 of the stored file's bytes, or `builtin` for the built-in Layout.
A new protocol selection clears every value too, and restarts the per-stream
and per-Device one-second rate windows.

A number appends one sample to the graph's ring and drops the oldest past 32. A
series replaces the ring. The redraw timer runs every 100 ms and redraws only
the graph Tiles whose values changed. A value past its `ttl_ms`, or every value
while the link is offline, is drawn at 60 percent opacity with the `--`
unavailable marker, and the graph stops advancing until a fresh value arrives.
A Tile with no value yet shows the marker and no line. Stale values are kept,
never set to zero. The line's geometry is in `ui_graph.c`. The UI preview uses
the same arithmetic, checked against `schema/live/graph-vectors.json` by both
test suites.

### Assets

With protocol 2 selected, the Device stores Assets by the SHA-256 of their bytes,
as the [protocol contract](../schema/protocol/link-v2.md#assets-and-bulk-transfers)
says. `v2.asset.stat` reports the space, `v2.asset.list` pages through the stored
Assets four at a time, `v2.asset.put` opens a transfer or answers
`present:true` for an Asset the Device already has, `v2.asset.end` commits it,
and `v2.asset.delete` removes one. This Firmware takes tdi1 icons only, so
`caps.asset_bytes` is 12300 and `caps.formats` is `["tdi1"]`.

An upload streams into `/storage/assets/upload.tmp` as its chunks arrive and is
hashed on the way, so no Asset is held whole in RAM. `v2.asset.end` checks the
size, the SHA-256 and the tdi1 header, then flushes the file and renames it to
`/storage/assets/<digit>/<sha256>.bin`, where `<digit>` is the id's first hex
digit. LittleFS finds a name by walking its directory, so the 16 shards keep each
walk short, and a delete removes a shard that it leaves empty. A cut upload never
publishes a partial Asset, and the next mount removes a leftover `upload.tmp`.
The Asset ids are longer than the 32-character path segments of the `store`
console command, so `store ls /storage/assets/<digit>` lists them, but
`store stat` and `store rm` cannot name one.

`free_bytes`, here and in `caps`, keeps 65536 bytes of the volume back for
Layouts, settings and LittleFS metadata, and an open upload reserves its size, so
Assets never fill the volume past what a Layout save needs. A put that does not
fit is refused with `full`. A delete is refused with `in-use` while a stored
Layout names the Asset as `sha256:<id>`.

The `td-link` CLI wraps them: `asset put <file>` packs a PNG into a 64 by 64
tdi1 with the app's icon packer, or sends a tdi1 file as it is, and skips the
upload when the Device has the Asset. `asset ls`, `asset rm <id>`, and
`asset df` list, remove, and report space.

The Firmware test app links the link dispatcher to exercise maximum-size hello
replies and NVS persistence. Its pinned UI dependencies and LVGL snapshot and
font settings match the components linked by the production Firmware.

### Asset admission accounting

The Asset admission budget is rebuilt once at mount. Each file is charged its
encoded size rounded up to 4,088-byte data blocks plus one 4,096-byte metadata
block; each directory is charged 8,192 bytes. Commits, removals and directory
changes update the running count under the store lock. `v2.asset.stat` reports
this conservative allocation budget, not LittleFS's block-walk measurement.
Open uploads reserve their full charge until commit or abort; headroom remains
65,536 bytes. LittleFS can still refuse a write, which returns `full`.
The console's `store` usage remains the actual LittleFS measurement.

Static schema 2 Tiles can reference `asset: "sha256:<id>"` with `icon: null`.
Firmware requires v2 negotiation and a validated tdi1 Asset before committing
the Layout. The renderer reads Assets from the internal sixteen-shard store.

### Photo frame and idle clock

`v2.idle.set` stores the photo frame's idle settings in NVS namespace
`td_device`, key `idle`: a 10-byte blob of version 1, `enabled`, then
`timeout_s` and `interval_s` as little-endian 32-bit integers. A missing or
unreadable blob reads as off, 60 s, and 15 s. The boot applies it after the
power gate starts, and a set applies at once.

With the photo frame on, the power gate never turns the screen off. After
`timeout_s` without a touch on a lit screen it clears every held key and raises
the idle face, which covers the Page. The face shows the latest picture while
the app is connected and a picture came within two intervals, and the clock
otherwise. The first touch takes the face down. The gate eats that contact
whole, so it clicks nothing, and the Page underneath is the one that was showing.
Turning the photo frame off takes the face down and the screen-off timeout
applies again.

A picture arrives as `v2.photo.put`, chunks, and `v2.photo.end`, in one PSRAM
buffer of at most 131072 bytes; nothing is written to the store. The end checks
the SHA-256, then decodes the JPEG on the link task with the ROM TJpgDec into a
PSRAM RGB565 buffer no larger than the screen, about 1.2 MB, and swaps it in
under the LVGL lock. The old picture is freed after the swap, so the peak is two
decoded pictures and one JPEG. The face frees its picture when it goes down.
The log line `idle: picture WxH from N bytes, decoded in T ms` gives the decode
time. TJpgDec takes baseline three-component JPEGs, so a grayscale or
progressive JPEG is refused with `format`.

`v2.clock.set` sets a volatile clock that counts on from the set by the
monotonic time. Until the first set after boot the clock reads `--:--`, dimmed.

`power state` adds `photo_idle` (the photo frame is on), `idle_face` (the face
is up), `photo_timeout_s`, `face_count` (times the face came up since boot), and
`face_wakes` (contacts eaten to take it down).

### GIF validation host tests

Both tests need Pillow, which the ESP-IDF Python environment doesn't ship.
Use any Python 3 with Pillow at the version CI pins
(`python -m pip install Pillow==13.0.0`, see `.github/workflows/firmware.yml`).
That can be a plain Python or a virtualenv of your own; or, in an ESP-IDF shell,
install the pin into the IDF environment, as CI does. Then, from the repository
root, run `python firmware/tools/test_gif.py` and `python firmware/tools/test_asset.py`.
Linux uses `CC` (default `cc`); Windows uses an x64 Visual Studio developer
terminal. Set `TD_TEST_OUT` to an existing local evidence directory to keep
compiled test libraries there. Resolve the locked dependencies first with
`idf.py -C firmware reconfigure` (a Firmware build also does this). The Asset
runner additionally uses the pinned LittleFS sources over a RAM block device to
check commit visibility, typed sync/close failures and handle cleanup. CI runs
these tests after the Firmware build resolves its dependencies.
`python firmware/tools/test_tile_geometry.py` compiles `ui_geometry.c` and
`ui_pages.c` and checks every case in `schema/live/tile-geometry-vectors.json`,
the Tile rects the Companion and the app also reproduce. These tests compile the
production validator and Asset store. The store harness substitutes the scheduler and storage accounting;
Windows directory enumeration is not exercised by this harness.

The GIF validator reads the temporary file as a stream, validates every LZW code,
counts the exact expanded pixels and checks every palette index. It stores each
LZW dictionary entry's length, first index and largest index, so validation needs
no canvas buffers. Its fixed workspace is about 16 KiB. It yields one scheduler
tick after at most 256 input-byte or LZW-code operations, including while checking
already committed Assets. Only a fully validated file reaches the atomic rename.
Size limits return `size`, malformed encoding returns `format` and allocation
failure returns `memory`. An interrupted commit leaves only the temporary file,
which normal mount cleanup removes.

The store recognizes normalized GIF89a up to 262144 encoded bytes, 128 by 128
pixels and 60 frames, with at least 100 ms per frame. GIF Layout references must
name GIF Assets; static references must name tdi1 Assets. Delete protection parses
both stored orientations, including escaped JSON Asset references. An unreadable
or invalid stored Layout prevents deletion. Production still advertises and
accepts only tdi1 on the link until `CONFIG_TD_GIF_CAPS` is enabled after playback
and the combined board proof. The C Layout parser already accepts the GIF schema
introduced by the contract step; this step tests its render kind and empty streams.

### Volume and Media protocol foundations

Run `python firmware/tools/test_media.py` in the same host environment as the
Asset tests. It compiles the production Layout and live-message parsers and the
input mailbox. The Asset runner also checks complete JPEG decode, dimensions,
truncation, unsupported encodings and reference-counted deletion protection on
both native files and LittleFS. Host JPEG tests use the pinned LVGL TJpgDec with
ROM-compatible format restrictions; the ESP32-S3 build uses ROM TJpgDec.

JPEG uploads are at most 32768 bytes and 256 by 256 pixels. The store syncs the
upload before full decode to a null sink without per-block sleeps, then commits it
atomically. The decoder unread-byte count locates EOI; trailing data is refused.
Presence and pin checks read only the stored JPEG header and size. JPEG is
for live images; static Tile Assets remain tdi1. The Layout parser accepts bar
and now-playing streams, with at most four image Tiles across all Pages.

Deletion returns `in-use` while any pin remains. Eight pin entries allow four
active covers plus four replacements. The link cancels queued input on a new
session or disconnect, and `td_link_press_part` emits control parts only in a
selected v2 session.

### Volume and Media renderers

A `bar` Tile draws, from `schema/live/bar-vectors.json`, the speaker glyph, its
label and its value in percent along the top, and the track along the bottom.
The fill is the accent, or ink-2 while `muted` is nonzero. A stale value keeps
its fill at 60 percent with `--` for its number. A bar with `input: true` holds a
press in its capture zone, so a drag on the bar never swipes the Page. A tap on
the track sends nothing. Once the finger passes the pager's slop, the bar draws
the finger's value and sends it with `td_link_input`, which coalesces moves to
one every 50 ms, then sends the end when the finger lifts or the press is lost.
After the lift the bar keeps the finger's value until a live value within 0.01
of it arrives, or 1.5 s pass. A stale or unset value disables the drag and
cancels one in progress, and so does a Layout or session change. A bar's drag
belongs to the live table's generation, which every Layout and session change
advances. The screen reads the generation and hands a move or an end to
`td_link_input` under one hold of the live-value lock, and a session reset
cancels the input mailbox after it advances the generation. A move or end from
before the change is therefore cancelled or never sent, and the first value of
the new session is drawn even when its `n` matches the old one. A tap on the
glyph of a bar that declares `muted` sends `press` with `part: "mute"`, but only
while the bar holds a fresh value of the current generation. Between a session
or Layout change and the redraw that follows it, the glyph names no part. Mute
is drawn only as the Companion reports it.

One module owns a Live tile touch from finger-down to what it queues for the
Companion: `ui_live_touch_t` in `td_ui_core/ui_live_bar.h`. It takes each press,
move, lift, fed value and tap with the live table's generation, decides whether
the touch can still act, and writes what it queues through a small output struct
(`ui_live_touch_out_t`: `input`, `cancel` and `part`). It drops a held
drag's queued input itself when the value goes stale, another press starts or the
touch resets. A part tap goes out with the generation it was named in, and the
link drops it when the table has moved on by the time it would be sent. The
LVGL handlers in `ui_live_tiles.c` and the link's input mailbox and press queue
are adapters around it, and its host tests in `test_ui_live_bar.c` drive it with
a fake link. A `hello` makes one call, `td_link_session_reset`, which discards
the open upload and resets the Live tile values and the touch.

A `now-playing` Tile draws, from `schema/live/now-playing-vectors.json`, the
cover, the title and the artist. A tall, wide Tile adds the previous,
play/pause and next controls in a row below them, and a tap on one sends its
part. A compact Tile is play/pause as a whole. State 0 draws "Nothing playing"
over the placeholder art, with no artist, no controls and no parts. A part is
named only for a fresh state of the current generation, so a tap between a
session or Layout change and the next redraw names none. A cover
larger than its art box draws its centred part, so its corners stay rounded.
Until the Tile hears a state it shows the `--` marker. Text is drawn as sent and
cut with an ellipsis to its column. Glyphs the font lacks draw as placeholder
boxes.

A cover is set in two phases, so no lock is held across a decode. The link task
checks the set under the live-value lock and, for an image the slot does not
already show, releases the lock. It pins the Asset with `td_asset_pin`, reads at
most 32768 bytes of it, and decodes it with ROM TJpgDec into an RGB565 buffer in
PSRAM. Then it commits the set under the lock again. The redraw timer swaps the
buffer onto the screen, frees the old one and logs
`live: cover <tile> shown t=<ms>`. A refusal at either phase frees the decode
and gives the pin back. A Layout or session change during the decode refuses the
set with `layout`. A heartbeat of the cover a slot already shows needs no decode.

The pin follows the live value. A slot gives its pin back when its image is
replaced or cleared, on a session reset, and on a Layout reload, before the link
handles its next message. A delete of an old cover therefore answers `in-use`
only while a slot still shows it. Unpins run with the live-value and LVGL locks
released, so the LVGL task never waits on the storage lock.

Taps come first. The touch and LVGL tasks never wait on a decode: the live-value
lock is held only to check, commit and copy values, to read the generation for a
tap on a part, and to drive the held bar's touch and send its value, and the storage
lock only to read the file. The link task runs at priority 3, below the LVGL
task at 6. The decode sleeps a tick after each row of JPEG blocks, 16 rows for a
256 by 256 cover, so the idle tasks run and feed the task watchdog. A decoded
cover is at most 256 by 256 pixels plus one spare row, about 128.5 KiB. At most
four are shown and four more wait to be drawn while one decodes, with its
32 KiB file buffer in PSRAM and a 4 KiB work area in internal RAM.

### GIF playback (test builds)

`CONFIG_TD_GIF_CAPS` also enables the GIF Tile renderer. It remains disabled in
the shipped configurations: real Devices still advertise only `tdi1`, zero GIF
frames, and no `anim`. Enable it only in a separate test configuration until the
combined hardware pass establishes the animation and tap budgets.

One priority-1 worker on core 1 owns playback: its file reads, decoding and
PSRAM allocations. It sleeps on its task notification until the earliest frame
deadline among playing Tiles. While nothing plays it sleeps until a Page, pause,
wake or Layout change notifies it. It checks cancellation at most every 256
input, dictionary or pixel operations and yields a scheduler tick when a slice
has consumed about 1 ms. No LVGL or shared state lock spans a decode or a file read. The LVGL timer
only consumes a ready descriptor and invalidates the Tile image. An expired
frame is discarded during decode, at publication, or when the UI consumes it;
each Tile has at most one pending frame, with no catch-up queue.

Each playing Tile retains its encoded bytes, a bounded decoder workspace, two
RGB565/A8 playback buffers and an immutable frame-0 poster. A Page change
cancels outgoing work and drops its playback buffers, encoded bytes and decoder
before starting incoming playback. Off-screen Pages, the idle face and
screen-off retain only posters and Tile metadata. A Layout reload decodes the
next Layout's posters on the reload task, outside the LVGL lock, while the shown
Layout keeps playing. Under the LVGL lock it then detaches the old images, waits
for the worker to release the old buffers, and builds the new Layout. If the
LVGL lock times out, the reload frees the staged posters and returns
`display.busy`, and the shown Layout keeps playing as before.

Slides, the idle face and screen-off show posters and pause playback. Wake and
the end of a slide restart from frame 0. `ui_gif_set_dimmed` supplies the same
pause for a future screen-dimming implementation. Any task may call it; the
LVGL timer applies it. The current power gate has active and off states only.
A Companion disconnect alone does not pause a GIF. A refused allocation shows
the poster and retries after 1 s or on the next Page activation. A file or
decode error keeps the Tile on its poster until the next Layout load. A Tile
whose poster could not be prepared uses its existing label rendering.

Layout commit checks every Page before writing it: at most four animated Tiles
and 65536 canvas pixels, counting repeated Assets once per Tile. Refusal returns
wire reason `size` and preserves the stored Layout.

Run `python -B firmware/tools/test_playback.py` and
`python -B firmware/tools/test_gif_ui.py` alongside the existing host tests.
The latter compiles the production worker/renderer against host scheduling and
LVGL substitutes and checks buffer ownership across concurrent Page changes,
slides, idle/off/dim states, and repeated Layout teardown. It also checks that a
reload whose LVGL lock times out keeps the shown Layout and leaks nothing, that
the worker stays asleep while every Tile is paused, and allocation retries. It does not measure
the panel or ESP32 timing. `test_gif.py` also runs 32 synthetic Companion outputs
through the production validator and decoder, comparing all decoded frames and
delays. Regenerate that corpus with
`python -B firmware/tools/generate_gif_fixtures.py`; this invokes the repository's
Rust normalizer in a temporary Cargo project using the app lockfile.

Tile `key` and `consumer` actions with `sink:"bluetooth"` bypass the Companion engine
and send press/release reports through the Bluetooth sink. Their modifiers are
independent of Companion modifier Tiles. Omitted sinks keep the Companion link;
normal Firmware rejects `bluetooth`. `test_ble.py` also drives Tile presses through
`ui_tile_click` and checks the encoded shortcut, media and release reports.
