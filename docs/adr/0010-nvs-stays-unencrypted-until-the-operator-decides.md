# NVS stays unencrypted until the operator decides

The Device keeps Pairings, and later its Wi-Fi credentials, in NVS. Both net-v2 ends hold the same long-term key (`LTK`), so a copy of the Device's NVS is a copy of the Device's identity ([net-v2](../../schema/protocol/net-v2.md#the-long-term-key-is-symmetric)). The radio build could encrypt NVS to make that dump harder. This record says why it doesn't yet, and what the choice would be.

## What ESP-IDF v6.1 offers

NVS encryption is XTS-AES over every entry. The ESP32-S3 can protect the XTS keys in two ways ([NVS Encryption](https://docs.espressif.com/projects/esp-idf/en/v6.1/esp32s3/api-reference/storage/nvs_encryption.html)):

- **HMAC peripheral scheme** (`CONFIG_NVS_SEC_KEY_PROTECT_USING_HMAC`, the default on the S3 once `CONFIG_NVS_ENCRYPTION` is on). The first `nvs_flash_init` generates an HMAC key and burns it into an eFuse key block (`CONFIG_NVS_SEC_HMAC_EFUSE_KEY_ID`, 0 to 5). The XTS keys are derived at run time and are never in flash. It needs no flash encryption. Software can't read the key back, so a flash dump alone doesn't open the NVS.
- **Flash-encryption scheme** (`CONFIG_NVS_SEC_KEY_PROTECT_USING_FLASH_ENC`). The XTS keys sit in an `nvs_keys` partition, which is safe only because flash encryption encrypts that partition. Without flash encryption those keys are plain flash and the scheme protects nothing. Flash encryption burns eFuses (the key, `SPI_BOOT_CRYPT_CNT` and, in release mode, the download-mode restrictions), and development mode allows only a few re-flashes with a plaintext image.

## Why not now

- Both schemes burn eFuses. A burn can't be undone, and a wrong key block or purpose can leave the Device unable to use that block again.
- Nothing in a build can make the burn conditional. Turning on `CONFIG_NVS_ENCRYPTION` makes every Device that boots the image burn a key on first boot, including the operator's only board.
- Flash encryption changes how the Device is serviced. `idf.py flash` must write images that the bootloader encrypts, and release mode closes the plaintext route that the recovery recipe in [docs/firmware.md](../firmware.md) relies on. The USB-Serial/JTAG recovery path is the project's safety net, so it isn't traded without a decision.
- Existing NVS content is plaintext. Switching schemes later needs a migration step (read the old entries, erase, write them encrypted). That is safe to write once, and not before the key scheme is chosen.
- What the Device stores today is a device name and the PHY calibration. Pairings arrive with the net-v2 tickets and Wi-Fi credentials with the Wi-Fi ticket, and the Wi-Fi driver's own NVS storage is off (`CONFIG_ESP_WIFI_NVS_ENABLED=n`). Until those land there is nothing secret to protect.

A safe, reversible setting doesn't exist: `NVS_ENCRYPTION` is either off, or it burns an eFuse.

## Decision

The radio build leaves NVS unencrypted, and the Firmware never burns an eFuse or enables flash encryption on its own. The build stays reversible, so a Device can always be reflashed over USB.

`firmware/tools/radio_guard.py` enforces this for both builds: the normal and the radio guard fail when `NVS_ENCRYPTION`, `SECURE_FLASH_ENC_ENABLED`, `SECURE_BOOT`, `SECURE_BOOT_V2_ENABLED` or `BOOTLOADER_APP_ANTI_ROLLBACK` is on, with a build-time unit test over the defaults files. A later change that enables one has to edit the guard and this record.

The consequence is stated where people read it: net-v2 says that a dumped NVS lets someone impersonate the Device and the app, and that the only mitigations are physical access and `net.forget`.

## Recommended path, for the operator to approve

If the operator wants Pairings protected at rest, use the HMAC scheme with a key the Device generates on its first boot:

1. Set `CONFIG_NVS_ENCRYPTION=y`, `CONFIG_NVS_SEC_KEY_PROTECT_USING_HMAC=y` and a fixed `CONFIG_NVS_SEC_HMAC_EFUSE_KEY_ID`, in `radio.defaults` only, so releases stay unchanged.
2. Handle `ESP_ERR_NVS_SEC_HMAC_KEY_BLK_ALREADY_USED` (the chosen key block holds another key) by failing loudly, not by picking another block.
3. Add a one-shot migration of the plaintext Pairings and name, and a clear USB console message before the first burn.
4. Prove it on a spare board, never the operator's Device first. Record that a burned Device can't return to plaintext NVS and that a board swap needs a fresh Pairing.
5. Replace the guard entry for `NVS_ENCRYPTION` with a check that the HMAC scheme and one key ID are set.

Flash encryption isn't recommended: it protects the whole flash, but it costs the recovery path and burns more eFuses, which the Pairings don't need.

## Considered options

- **Enable the HMAC scheme in the radio build now.** Closes the dump, but burns an eFuse on the first boot of any radio image, with no operator decision.
- **Enable flash encryption and the key-partition scheme.** Larger protection and larger cost: several eFuses, a changed flash and recovery procedure.
- **Enable the key-partition scheme without flash encryption.** Costs nothing and protects nothing, since the keys sit beside the data. Rejected as a false sense of security.
- **Encrypt Pairings in the application with a key kept in flash.** Same weakness as the previous option.
- **Leave NVS plain, state the consequence, and guard against an accidental burn.** Chosen.
