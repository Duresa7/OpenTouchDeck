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

/* Commands the net task runs for USB and the console. Each waits for the net task. */
typedef enum {
    TD_NETLINK_CMD_OK,
    /* The service isn't running or didn't answer in time. */
    TD_NETLINK_CMD_DOWN,
    TD_NETLINK_CMD_LOCKED,
    TD_NETLINK_CMD_FULL,
    TD_NETLINK_CMD_STORAGE,
} td_netlink_cmd_result_t;

td_netlink_cmd_result_t td_netlink_pair(void);
/* Erases every Pairing; removed receives how many there were. */
td_netlink_cmd_result_t td_netlink_forget_all(uint8_t *removed);

/* The net.state fields (schema/protocol/net-v2.md). Any task. */
void td_netlink_state_fields(cJSON *reply);
