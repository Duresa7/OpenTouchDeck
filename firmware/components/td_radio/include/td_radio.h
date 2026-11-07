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
 * td_radio_init sets up the Wi-Fi station logic (td_wifi.h): it reads the stored network, answers the `wifi.*` link
 * messages and masks the password the `wifi set` console command takes. Call it after the console commands are
 * registered and before td_link_start and td_console_start.
 *
 * td_radio_start starts the Wi-Fi station, which then joins the stored network, if there is one, and joins it again
 * after it drops. It never scans on its own. BLE is compiled in and stays dormant: nothing starts the controller or
 * the NimBLE host, so the Device never advertises, scans or connects. It also shows the Wi-Fi indicator, and thirty
 * seconds later (CONFIG_TD_RADIO_STEADY_MS) it logs the steady-state internal heap against the floor in
 * CONFIG_TD_RADIO_INTERNAL_FLOOR.
 *
 * Call td_radio_start after the display and touch started, so the panel's internal buffers come first.
 */
esp_err_t td_radio_init(void);
esp_err_t td_radio_start(void);

/* `radio state`: the build, the station, BLE, and the internal heap against the floor. `wifi`: see td_wifi.h. */
const esp_console_cmd_t *td_radio_console_commands(size_t *count);

#ifdef __cplusplus
}
#endif
