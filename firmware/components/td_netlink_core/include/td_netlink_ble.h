#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * net-v2's BLE item rules, without a BLE stack: the fragment reassembler and the Device's flow credit. The BLE carrier
 * (#278) runs them; the host tests run the shared vectors through them now.
 */

#define TD_NETLINK_ATT_MTU_MIN 23
#define TD_NETLINK_ATT_MTU_MAX 517
/* The largest fragment a receiver that doesn't know its ATT_MTU accepts. */
#define TD_NETLINK_FRAGMENT_UNKNOWN_MAX (TD_NETLINK_ATT_MTU_MAX - 3)
#define TD_NETLINK_FRAGMENT_MIDDLE_MIN 20
#define TD_NETLINK_CREDIT_MIN 1050u
#define TD_NETLINK_CREDIT_RECOMMENDED 2100u

typedef enum {
    TD_NETLINK_FRAG_MORE,
    TD_NETLINK_FRAG_ITEM,
    TD_NETLINK_FRAG_CLOSE,
} td_netlink_frag_t;

typedef struct {
    uint16_t max_fragment;
    bool records;
    uint16_t f;
    bool pending;
    uint16_t need;
    uint16_t have;
    bool closed;
    uint8_t *buf;
    size_t cap;
} td_netlink_reasm_t;

/* max_fragment is the receiver's ATT_MTU - 3, or TD_NETLINK_FRAGMENT_UNKNOWN_MAX. buf holds one whole item. */
void td_netlink_reasm_init(td_netlink_reasm_t *r, uint16_t max_fragment, uint8_t *buf, size_t cap);
/* After session.ready, items are records. False when it falls inside an item, which closes. */
bool td_netlink_reasm_records(td_netlink_reasm_t *r);
/* One rx write. ITEM sets *item to the whole item, prefix included, until the next push. CLOSE ends the carrier
 * connection, and every later push closes too. */
td_netlink_frag_t td_netlink_reasm_push(td_netlink_reasm_t *r, const uint8_t *chunk, size_t len, const uint8_t **item,
                                        size_t *item_len);

typedef enum {
    TD_NETLINK_NOTIFY_TX,
    TD_NETLINK_NOTIFY_FLOW,
} td_netlink_notify_t;

typedef struct {
    uint32_t initial;
    bool tx;
    bool flow;
    bool started;
    bool closed;
    uint64_t limit;
    uint64_t written;
} td_netlink_credit_t;

/* initial is at least TD_NETLINK_CREDIT_MIN. */
void td_netlink_credit_init(td_netlink_credit_t *c, uint32_t initial);
/* Notifications enabled on tx or flow. Sets *value to the first u64be credit and returns true when this starts the
 * carrier connection. */
bool td_netlink_credit_enable(td_netlink_credit_t *c, td_netlink_notify_t which, uint8_t value[8]);
/* Notifications disabled: the carrier connection ends. */
void td_netlink_credit_disable(td_netlink_credit_t *c);
/* An rx write of len bytes. False ends the carrier connection: a write before it started or beyond the credit. */
bool td_netlink_credit_write(td_netlink_credit_t *c, size_t len);
/* Raises the limit. False for a lower one. */
bool td_netlink_credit_grant(td_netlink_credit_t *c, uint64_t limit, uint8_t value[8]);

#ifdef __cplusplus
}
#endif
