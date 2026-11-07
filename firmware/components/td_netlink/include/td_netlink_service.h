#pragma once

#include <stdbool.h>
#include <stddef.h>
#include "cJSON.h"
#include "esp_console.h"
#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * net-v2's TCP service (schema/protocol/net-v2.md), in the radio build only. It listens on TCP 47820, admits one owner
 * and refuses or takes over other connections, runs pairing and sessions through td_netlink_core, keeps the Pairings
 * and the pairing failure counter in NVS, shows the pairing screen, and adds a network carrier named "wifi" to the
 * link (td_link_carrier.h). Pairing windows open only from USB: the console's `net pair` or the USB-only `net.pair`.
 */

/* Loads the stored Pairings, opens the listener and starts the net task, whose stack and state live in PSRAM. Call it
 * after td_link_start and once lwIP is up. It never waits for a client. */
esp_err_t td_netlink_start(void);

/* `net state | pair | forget`. Register before td_console_start. */
const esp_console_cmd_t *td_netlink_console_commands(size_t *count);

#ifdef __cplusplus
}
#endif
