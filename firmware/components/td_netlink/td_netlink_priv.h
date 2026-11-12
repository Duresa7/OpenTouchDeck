#pragma once

#include <stdbool.h>
#include <stdint.h>
#include "cJSON.h"
#include "td_netlink.h"

/* NVS: the Pairings as one blob, and pair_failures with the lockout's time left. Each save commits before it returns
 * true. NVS must be initialised. Run them on a task with an internal stack. */
bool td_netlink_nvs_load(td_netlink_pairings_t *pairings, uint32_t *pair_failures, uint32_t *lock_left_ms);
bool td_netlink_nvs_save_pairings(const td_netlink_pairings_t *pairings);
bool td_netlink_nvs_save_failures(uint32_t pair_failures, uint32_t lock_left_ms);

/* net.pair, net.list or net.forget (td_netlink_manage.h), run on the net task, which owns the Pairings. The reply holds
 * `ok` already and gets the answer. Waits for the net task; false when the service isn't running or didn't take the
 * request in time, and then the reply is untouched. */
bool td_netlink_manage_request(const cJSON *request, cJSON *reply);

/* The net.state fields (schema/protocol/net-v2.md). Any task. */
void td_netlink_state_fields(cJSON *reply);
