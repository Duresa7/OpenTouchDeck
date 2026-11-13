# Heartbeat

While running, the firmware prints a numbered `alive` line with the current free heap every 2 seconds, so a user can see at a glance that it hasn't hung, rebooted, or started leaking memory.

## Sub-features

- `heartbeat-tick` prints `alive N` with N counting up from 0 after each boot.
- `heartbeat-interval` spaces ticks about 2 seconds apart.
- `heartbeat-heap` reports `free heap <bytes> bytes` on every tick.

## How to get to it (user POV)

- Open a serial monitor on the board's port any time after boot and watch the log.

## Driving it with touchdeck.ps1

Preconditions:

- `$TD serial doctor` exits `0`.
- Evidence directory `E=.verify/<YYYYMMDD-HHMMSS>-heartbeat` is new, and Doctor's JSON is saved there as `doctor.json`.

- **Watch from boot.** Reset and record 10 seconds. Run `$TD serial capture --out $E --seconds 10 --expect "touchdeck: alive 0, free heap \d+ bytes" --expect "touchdeck: alive 1, free heap \d+ bytes" --expect "touchdeck: alive 2, free heap \d+ bytes" --expect "touchdeck: alive 3, free heap \d+ bytes"`. Exit `0`, with ticks 0 through 3 in order.
- **Interval.** Read the device timestamp `I (<ms>)` from each matched `line` in `$E/summary.json`. Each gap between consecutive ticks is between 1.8 and 2.2 seconds by that clock. The host `t` is only a cross-check, because USB reads arrive in batches and can put a gap near 1.82 s.
- **No reboot mid-run.** Run `grep -c "TouchDeck firmware starting" $E/serial.log`. The count is `1`, so the counter never restarted.
- **Attach without reset.** Watch a board that's already running. Run `$TD serial capture --out $E/attached --seconds 5 --no-reset --expect "touchdeck: alive \d+, free heap \d+ bytes"`. Exit `0`, with N above 0 when the board has been up longer than 2 seconds.
- **Proof.** Keep `$E/doctor.json`, `$E/serial.log`, `$E/summary.json`, and `$E/attached/`. Note the free-heap values in the report. Free heap dropping steadily from tick to tick suggests a leak, so call it out.

## Gotchas

- `alive 0` prints right after the boot report, under a second after reset. A capture with `--no-reset` started later will never see tick 0.
- Free heap varies by a few hundred bytes between builds and IDF versions. Assert the format, not an exact number.
