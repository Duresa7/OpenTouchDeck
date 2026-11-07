#pragma once

#include <stddef.h>
#include "esp_console.h"
#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The radio build's radios. Only the radio build (idf.py -DTD_RADIO_BUILD=1) has this component; the normal build
 * never links it.
 *
 * td_radio_start starts the Wi-Fi station and joins no network: nothing sets a network or calls esp_wifi_connect, so
 * it does not scan or associate. BLE is compiled in and stays dormant: nothing starts the controller or the NimBLE
 * host, so the Device never advertises, scans or connects. Thirty seconds later (CONFIG_TD_RADIO_STEADY_MS) it logs
 * the steady-state internal heap against the floor in CONFIG_TD_RADIO_INTERNAL_FLOOR.
 *
 * Call it after the display and touch started, so the panel's internal buffers come first.
 */
esp_err_t td_radio_start(void);

/* `radio state`: the build, the station, BLE, and the internal heap against the floor. */
const esp_console_cmd_t *td_radio_console_commands(size_t *count);

#ifdef __cplusplus
}
#endif
