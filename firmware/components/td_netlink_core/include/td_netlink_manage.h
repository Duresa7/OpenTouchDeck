#pragma once

#include <stdbool.h>
#include <stdint.h>
#include "cJSON.h"
#include "td_netlink.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * net-v2's USB-only Pairing messages on a deck: net.pair, net.list and net.forget (schema/protocol/net-v2.md, "Pairing
 * slots" and "Storage, management and secrets"). The link refuses them on a network carrier before they get here, so
 * whoever calls this has a USB request. The host tests run the shared `management` vector through it.
 */

#define TD_NETLINK_APP_ID_LEN (2 * TD_NETLINK_COMPANION_ID_LEN)
#define TD_NETLINK_FULL_ERROR "Four apps are paired. Forget one first."

typedef struct {
    /* Adds the net.state fields the deck doesn't hold: listening, port, ble and session. */
    void (*base_fields)(void *ctx, cJSON *reply);
    /* Called before a forget erases anything: ends the network session of companion_id, or, when it is NULL, every
     * network session and every connection waiting to take over. */
    void (*end_sessions)(void *ctx, const uint8_t *companion_id);
    void *ctx;
} td_netlink_manage_env_t;

/* Answers one net.pair, net.list or net.forget. The reply already holds `m` and `ok:true`. A refusal turns `ok` false
 * and adds only `reason` and `error`; a success adds the net.state fields and the message's own (`free` and `pairings`,
 * or `removed`). */
void td_netlink_manage(td_netlink_deck_t *deck, const cJSON *request, cJSON *reply,
                       const td_netlink_manage_env_t *env);

/* net.state's fields that the deck holds: paired, pairing, pair_failures and locked_ms. */
void td_netlink_deck_fields(const td_netlink_deck_t *deck, cJSON *reply);

/* app_id: companion_id as 32 lowercase hex characters, NUL-terminated. */
void td_netlink_app_id(const uint8_t companion_id[TD_NETLINK_COMPANION_ID_LEN], char out[TD_NETLINK_APP_ID_LEN + 1]);
/* False unless text is exactly 32 lowercase hex characters. */
bool td_netlink_parse_app_id(const char *text, uint8_t out[TD_NETLINK_COMPANION_ID_LEN]);

#ifdef __cplusplus
}
#endif
