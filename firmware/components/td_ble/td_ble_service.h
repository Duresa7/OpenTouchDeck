#pragma once

#include "cJSON.h"
#include "td_ble_core.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The serialized way into the core, for the link messages and the console. td_ble_nimble.c takes its lock around each
 * call; the host tests run them against a core with a fake port.
 */
td_ble_result_t td_ble_set_enabled(bool on);
td_ble_result_t td_ble_pair(bool open);
td_ble_result_t td_ble_forget(const td_ble_addr_t *peer, size_t *forgotten);
void td_ble_status(td_ble_status_t *out);

/* The `ble.*` link messages (td_ble_msgs.c). */
void td_ble_message(const cJSON *request, cJSON *reply);
/* Adds `ble_hid`, the most bonded hosts, to the caps of a v2 hello. */
void td_ble_caps(cJSON *caps);
/* "aa:bb:cc:dd:ee:ff", most significant byte first. */
void td_ble_addr_format(const td_ble_addr_t *addr, char out[18]);
/* The inverse, for either address type: only the six bytes are read. */
bool td_ble_addr_parse(const char *text, td_ble_addr_t *out);

#ifdef __cplusplus
}
#endif
