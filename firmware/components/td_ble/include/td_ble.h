#pragma once

#include <stddef.h>
#include "esp_console.h"
#include "esp_err.h"
#include "key_sink.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The radio build's Bluetooth keyboard: a NimBLE HID-over-GATT peripheral with a keyboard and a consumer (media)
 * report, for a phone, a tablet or another computer. Only the radio build has this component.
 *
 * BLE is off until the person turns it on with `ble.set` or `ble on`. The setting persists in NVS (namespace td_ble,
 * key enabled), and only a stored "on" starts the controller and the host at boot. Pairing happens only in a window the
 * person opens over USB. See docs/firmware.md, Bluetooth keyboard, and schema/protocol/link-v2.md for the messages.
 */

/* Sets up the state and adds the `ble.*` link messages, which are USB-only, and the `ble_hid` capability. Call it
 * before td_link_start. */
esp_err_t td_ble_init(void);

/* Starts BLE when the stored setting is on, and does nothing else when it is off. Call it after td_ble_init, once the
 * display, touch and Wi-Fi have their internal RAM. */
esp_err_t td_ble_boot(void);

/* Every key report goes to a bonded, subscribed host while one is connected, and nowhere while none is. */
key_sink_t td_ble_key_sink(void);

/* `ble state|on|off|pair|close|forget [address]`. */
const esp_console_cmd_t *td_ble_console_commands(size_t *count);

/* For `radio state`: "off", "starting", "on" or "failed". */
const char *td_ble_state_name(void);

#ifdef __cplusplus
}
#endif
