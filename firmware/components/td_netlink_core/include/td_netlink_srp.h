#pragma once

#include <stddef.h>
#include <stdint.h>

#define TD_NETLINK_SRP_LEN 384
#define TD_NETLINK_SALT_LEN 16
#define TD_NETLINK_EXP_LEN 32
#define TD_NETLINK_CODE_LEN 6

typedef enum {
    TD_NETLINK_SRP_OK,
    TD_NETLINK_SRP_BAD_PUBLIC,
    TD_NETLINK_SRP_FAIL,
} td_netlink_srp_result_t;

typedef struct {
    const uint8_t *identity;
    size_t identity_len;
    const char *code;
    const uint8_t *salt;
    const uint8_t *b;
    const uint8_t *A;
} td_netlink_srp_in_t;

/* The deck's whole SRP-6a step from net-v1: v, B, u, S, and K_srp = H512(MIN(S)).
 * BAD_PUBLIC means A is outside 0 < A < N. FAIL covers a zero b, B, u, or S and
 * any arithmetic or allocation failure. Intermediates are erased either way. */
td_netlink_srp_result_t td_netlink_srp_server(const td_netlink_srp_in_t *in, uint8_t B[TD_NETLINK_SRP_LEN], uint8_t key[64]);
