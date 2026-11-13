# Boot report

After power-on or reset, the firmware announces itself on the USB serial console and reports the chip, flash size, and PSRAM it found, so a user can tell the board is running TouchDeck and that its memory came up.

## Sub-features

- `boot-banner` prints `TouchDeck firmware starting` once per boot.
- `boot-chip` reports the target (`esp32s3`), core count, and silicon revision.
- `boot-flash` reports the detected flash size in MB.
- `boot-psram` reports the PSRAM size in KB. 0 means PSRAM failed to start, and the display will not be able to run.
- `boot-partition` (M1-02 and later) reports the running app partition by label and offset, `Running partition ota_0 at 0x40000`. Firmware before M1-02 has no such line, so don't require it there.

## How to get to it (user POV)

- Plug the board into USB (power-on) and open a serial monitor on its port.
- Press the board's RESET button with a serial monitor open.
- Flash new firmware. The flash tool resets the board when it finishes.

## Driving it with touchdeck.ps1

Preconditions:

- `$TD serial doctor` exits `0` (`build_matches`, `banner_seen`, `alive_seen` all true).
- Evidence directory `E=.verify/<YYYYMMDD-HHMMSS>-boot-report` is new, and Doctor's JSON is saved there as `doctor.json`.

- **Power-on equivalent.** Reset the board and record its log. Run `$TD serial capture --out $E --seconds 8 --stop-when-matched --expect "touchdeck: TouchDeck firmware starting" --expect "touchdeck: Chip: esp32s3, 2 core\(s\), revision v\d+\.\d+" --expect "touchdeck: Flash: 16 MB" --expect "touchdeck: PSRAM: 8192 KB"`. Exit `0`, and `summary.json` shows all four expectations matched in this order within the first few seconds.
- **Running partition (M1-02 and later).** Repeat the capture with one more expectation after PSRAM. Add `--expect "touchdeck: Running partition ota_0 at 0x40000"`. Exit `0`, and the bootloader's partition table earlier in `serial.log` lists `nvs`, `otadata`, `phy_init`, `coredump`, `ota_0`, `ota_1`, and `storage`.
- **Exactly one banner per boot.** Count banners in the transcript. Run `grep -c "TouchDeck firmware starting" $E/serial.log`. The count is `1`. More than one means the firmware is boot-looping.
- **Proof.** Keep `$E/doctor.json`, `$E/serial.log`, and `$E/summary.json`. Together they show the build identity, the reset (`rst:` line from the ROM), and the four report lines.

## Gotchas

- The ROM and second-stage bootloader lines (`ESP-ROM:`, `I (..) boot:`) come before the app's report. Don't mistake the bootloader's `SPI Flash Size : 16MB` for `boot-flash`. Match the `touchdeck:` tag.
- `--stop-when-matched` ends the capture early. For the boot-loop check, rerun without it for at least 10 seconds.
- `PSRAM: 0 KB` or a crash right after `octal_psram:` lines points at the PSRAM settings in `sdkconfig.defaults` (octal, 80 MHz). It's a real failure, so report it.
