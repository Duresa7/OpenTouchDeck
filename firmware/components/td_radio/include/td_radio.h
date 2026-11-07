#pragma once

#include <stddef.h>
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
 * td_radio_init sets up the Bluetooth keyboard (td_ble.h) and adds its USB-only `ble.*` link messages. Call it before
 * td_link_start. It starts no radio.
 *
 * td_radio_start starts the Wi-Fi station and joins no network: nothing sets a network or calls esp_wifi_connect, so
 * it does not scan or associate. Then it starts BLE only if the stored Bluetooth setting is on, which it is not until
 * the person turns it on: a fresh Device never advertises. BLE never scans or starts a connection. Thirty seconds later
 * (CONFIG_TD_RADIO_STEADY_MS) it logs the steady-state internal heap against the floor in
 * CONFIG_TD_RADIO_INTERNAL_FLOOR.
 *
 * Call it after the display and touch started, so the panel's internal buffers come first.
 */
esp_err_t td_radio_init(void);
esp_err_t td_radio_start(void);

/* Key reports for a bonded Bluetooth host. Until Tiles choose their target (#277), every report goes there too while
 * one is connected. */
key_sink_t td_radio_key_sink(void);

/* `radio state`: the build, the station, BLE, and the internal heap against the floor. `ble`: the Bluetooth
 * keyboard. */
const esp_console_cmd_t *td_radio_console_commands(size_t *count);

#ifdef __cplusplus
}
#endif
