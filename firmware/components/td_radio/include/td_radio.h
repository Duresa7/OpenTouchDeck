#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "esp_console.h"
#include "esp_err.h"
#include "key_sink.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The radio build's radios. Only the radio build (idf.py -DTD_RADIO_BUILD=1) has this component; the normal build
 * never links it.
 *
 * td_radio_init sets up the Bluetooth keyboard (td_ble.h) and the Wi-Fi station logic (td_wifi.h): it reads the
 * stored network, adds the USB-only `ble.*` and `wifi.*` link messages (td_link_extend) and masks the password the
 * `wifi set` console command takes. It sets up every part even when one fails, logs what failed and returns the first
 * error. Call it after the console commands are registered and before td_link_start and td_console_start. It starts
 * no radio.
 *
 * td_radio_start starts the Wi-Fi station, which then joins the stored network, if there is one, and joins it again
 * after it drops. It never scans on its own. Then it starts BLE only if the stored Bluetooth setting is on, which it is
 * not until the person turns it on: a fresh Device never advertises. BLE never scans or starts a connection. It also
 * shows the Wi-Fi indicator, and thirty seconds later (CONFIG_TD_RADIO_STEADY_MS) it logs the steady-state internal
 * heap against the floor in CONFIG_TD_RADIO_INTERNAL_FLOOR.
 *
 * Call td_radio_start after the display and touch started, so the panel's internal buffers come first.
 */
esp_err_t td_radio_init(void);
esp_err_t td_radio_start(void);

/* The explicit HID Tile destination, sending only while a bonded Bluetooth host is connected. */
key_sink_t td_radio_key_sink(void);

/* Where the Bluetooth pairing passkey goes on screen (td_ble_set_passkey_hook): show true with the six-digit code, or
 * false to take it down. The hook must not block. */
void td_radio_set_passkey_hook(void (*hook)(bool show, uint32_t passkey));

/* `radio state`: the build, the station, BLE, and the internal heap against the floor. `ble`: the Bluetooth
 * keyboard. `wifi`: see td_wifi.h. */
const esp_console_cmd_t *td_radio_console_commands(size_t *count);

#ifdef __cplusplus
}
#endif
