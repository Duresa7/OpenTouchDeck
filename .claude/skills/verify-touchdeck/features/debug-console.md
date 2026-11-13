# Debug console

From M1-03 on, the firmware runs a line console on the USB-Serial/JTAG port with the prompt `td> `. A developer or the harness types a command and gets one machine-readable reply line, so the board's identity, memory, and uptime can be read without a reset.

## Sub-features

- `console-version` replies with the app descriptor, including `project_name` and `elf_sha256`.
- `console-heap` replies with `free_heap` and `min_free_heap` in bytes.
- `console-uptime` replies with the time since boot.
- `console-help` lists the registered commands. On M1-03 that is exactly `help`, `version`, `heap`, `uptime`, and `reboot`. From M1-04 on it is those five followed by `screenshot`, `perf`, and `backlight`, in that order. From M1-05 on, `ui` and `touch` follow them, in that order, and from M1-09 on `keys` follows those. [Display](./display.md), [Touch](./touch.md), and [Keys](./keys.md) cover them.
- `console-reboot` replies, then restarts the board.
- `console-errors` answers an unknown command or an over-long line with `ok` false and an `error`, and keeps running. The reply names the word the console split off the line with `esp_console_split_argv`, which splits on spaces only and honors double quotes and backslash escapes.
- `console-overflow` reports input it had no room to queue, as `{"cmd":"console","ok":false,"error":"input overflow","dropped_bytes":N}`. The console queues 2047 bytes while a command runs. It drops one read of up to 64 bytes and every byte through the next CR or LF, so a line that a drop cut never runs, and `dropped_bytes` counts all of those bytes.

## How to get to it (user POV)

- Open a serial terminal on the board's port and type a command at `td> `.
- Run `$TD serial cmd "<line>"`, which does the same and prints the reply's JSON.

## Driving it with touchdeck.ps1

Preconditions:

- The build under test is M1-03 or later, and `$TD serial doctor` exits `0` with `build_matches` true.
- Evidence directory `E=<absolute path>/.verify/<YYYYMMDD-HHMMSS>-debug-console` is new, and Doctor's JSON is saved there as `doctor.json`.

- **Identity.** Run `$TD serial cmd "version" > $E/version.json`. Exit `0`, `project_name` is `touchdeck`, and `elf_sha256` equals `local_build.elf_sha256` in `doctor.json`.
- **Command list.** Run `$TD serial cmd "help"`. Exit `0`, and `commands` is `["help", "version", "heap", "uptime", "reboot"]` on M1-03, `["help", "version", "heap", "uptime", "reboot", "screenshot", "perf", "backlight"]` on M1-04, `["help", "version", "heap", "uptime", "reboot", "screenshot", "perf", "backlight", "ui", "touch"]` on M1-05, `["help", "version", "heap", "uptime", "reboot", "screenshot", "perf", "backlight", "ui", "touch", "keys"]` from M1-09 to M1-11, and `["help", "version", "heap", "uptime", "reboot", "screenshot", "perf", "backlight", "ui", "touch", "keys", "power"]` from M1-12 on.
- **Uptime moves.** Run `$TD serial cmd "uptime" --repeat 2 --interval 3`. Both replies have `ok` true, and the two values differ by about 3 seconds.
- **Unknown command.** Run `$TD serial cmd "nosuchcmd"`. Exit `1`, and the reply has `ok` false with an `error`. A following `$TD serial cmd "version"` exits `0`.
- **Reboot.** Run `$TD serial capture --out $E/reboot --seconds 10 --no-reset --send "reboot" --expect '@td \{"cmd":"reboot","ok":true\}' --expect "touchdeck: TouchDeck firmware starting" --expect "touchdeck: alive 0"`. Exit `0`. The banner and `alive 0` after the reply prove the restart.
- **Long line.** In PowerShell, run `$TD serial capture --out $E/long-line --seconds 6 --no-reset --send ('x' * 300) --send "version" --expect '"error":"line too long"' --expect '@td \{"cmd":"version","ok":true'`. Exit `0`, the transcript shows both `>> sent` markers, and `serial.log` has neither `TouchDeck firmware starting` nor `Guru Meditation`. `$TD serial cmd ('x' * 300)` alone exits `1` with the same error, because the reply echoes only the first 63 characters of the word and `cmd` accepts that prefix.
- **Tab stays in the word.** In PowerShell, run `$TD serial capture --out $E/tab --seconds 3 --no-reset --send ("version" + [char]9 + "x") --send "version" --expect '@td \{"cmd":"version\\tx","ok":false,"error":"unknown command"\}' --expect '@td \{"cmd":"version","ok":true'`. Exit `0`. The first reply names `version\tx`, the word the console ran, not `version`.
- **Pipelined burst.** In PowerShell, build `$s = 1..50 | ForEach-Object { '--send', 'version' }` and `$x = 1..50 | ForEach-Object { '--expect', '@td \{"cmd":"version","ok":true' }`, then run `$TD serial capture --out $E/pipelined --seconds 8 --no-reset @s @x`. Exit `0` means 50 replies matched in order. `serial.log` has no `input overflow` line. Without the fifty `--expect`, `capture` exits `0` on any count, so never drop them.
- **Overflow is reported.** In PowerShell, build `$p = (@('version') * 600) -join [char]10` and run `$TD serial capture --out $E/overflow --seconds 8 --no-reset --send $p --send "version"`. That sends 601 lines of 8 bytes, 4,808 bytes in all. Multiply the `version` replies by 8 and add the `dropped_bytes` of every `input overflow` line. The sum is 4,808. A shortfall means the firmware lost input without saying so. For any line lengths, the sum is exact only when no line is blank and every line ends in one CR or one LF. With CR LF endings it can be off by one byte per overflow, short or over. One loss can print several `input overflow` lines, so add every `dropped_bytes`. The USB driver underneath can still lose a 64-byte packet uncounted if `td_rx` is starved, so a shortfall is a finding, not proof of a firmware counting bug.
- **No glued line after an overflow.** In PowerShell, build `$p = (@('heap') * 1000) -join [char]10` and run `$TD serial capture --out $E/glue --seconds 8 --no-reset --send $p`, then `$TD serial cmd "version"`. The capture sends 5,000 bytes, which is not a multiple of 64. Multiply the `heap` replies by 5 and add the `dropped_bytes` of every `input overflow` line. The sum is 5,000, every reply is named `heap`, and `cmd` exits `0`. Before the resync fix, a `he` fragment survived the drop, the sum fell 2 short, and the next `version` ran as `heversion`, so `cmd` timed out with exit `2`. For lines of mixed lengths, send a unique command name on each line, so that a glued line is a name that was never sent.
- **Under display load (M1-04 on).** Run the overflow, no-glued-line, and pipelined recipes above with `--send "perf fps 10"` ahead of the burst and `--seconds 14`, so each measurement ends before the next run starts and the display redraws the whole screen during the burst. The sums stay 4,808 and 5,000, the pipelined run still matches 50 of 50, and every `perf` reply is `ok` true. Do not count `td> ` echo lines or log lines in a sum. Replies are the only output that is guaranteed to arrive.
- **Round trip.** Run `$TD serial cmd "version" --repeat 50 --time 2> $E/rtt.txt`. Every reply is identical, and the p95 of the `rtt_ms` lines is under 50 ms.

## Gotchas

- `cmd` never resets the board. Run it after Doctor or a capture, and expect heartbeat lines around the reply. `cmd` skips them.
- `cmd` ignores `input overflow` lines because their `cmd` is `console`. When `cmd` times out after a burst, look for one in a `capture`.
- Log lines that arrive while the console waits start with `td> `, as in `td> I (4012) touchdeck: alive 1, ...`. Recipes search the line, so they still match, but don't anchor a regex with `^`.
- `--send` in the reboot capture uses `--no-reset` on purpose. A reset first would race the send.
- `free_heap` counts PSRAM, so it reads about 8.7 MB. Compare it with the `alive` line's figure, not with internal RAM.
- Firmware before M1-03 has no console. `cmd` exits `2` on it within the timeout, which is the expected result there, not a harness fault.
