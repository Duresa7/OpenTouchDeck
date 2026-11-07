#pragma once

#include "td_netlink_crypto.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* net-v2 records: one complete link frame body sealed with AES-256-GCM under a per-direction key and counter. */

#define TD_NETLINK_PLAIN_MIN 8
#define TD_NETLINK_PLAIN_MAX 1032
#define TD_NETLINK_SEALED_MIN (TD_NETLINK_PLAIN_MIN + TD_NETLINK_TAG_LEN)
#define TD_NETLINK_SEALED_MAX (TD_NETLINK_PLAIN_MAX + TD_NETLINK_TAG_LEN)
#define TD_NETLINK_RECORD_WIRE_MAX (2 + TD_NETLINK_SEALED_MAX)
#define TD_NETLINK_COUNTER_LIMIT (UINT64_C(1) << 32)

typedef enum {
    TD_NETLINK_C2D = 0,
    TD_NETLINK_D2C = 1,
} td_netlink_direction_t;

/* One direction of a session. A failure closes it for good and erases the key. */
typedef struct {
    uint8_t key[TD_NETLINK_KEY_LEN];
    uint64_t counter;
    uint8_t direction;
    bool closed;
} td_netlink_dir_t;

void td_netlink_dir_init(td_netlink_dir_t *dir, td_netlink_direction_t direction, const uint8_t key[TD_NETLINK_KEY_LEN]);
void td_netlink_dir_close(td_netlink_dir_t *dir);
/* body is one complete link frame body. wire receives u16be(len) || sealed, 2 + len + 16 bytes. */
bool td_netlink_record_seal(td_netlink_dir_t *dir, const uint8_t *body, size_t len, uint8_t *wire);
/* sealed is the record after its length prefix. body receives sealed_len - 16 bytes, exposed only after the tag and the
 * whole link frame body check out. */
bool td_netlink_record_open(td_netlink_dir_t *dir, const uint8_t *sealed, size_t sealed_len, uint8_t *body);

#ifdef __cplusplus
}
#endif
