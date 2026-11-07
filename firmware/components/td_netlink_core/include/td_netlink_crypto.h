#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* net-v2's primitives (schema/protocol/net-v2.md), through Mbed TLS PSA. Every function returns false when PSA reports
 * any failure, with its output left unspecified. */

#define TD_NETLINK_H_LEN 32
#define TD_NETLINK_H512_LEN 64
#define TD_NETLINK_KEY_LEN 32
#define TD_NETLINK_TAG_LEN 16
#define TD_NETLINK_NONCE_LEN 12
#define TD_NETLINK_SHARE_LEN 32

typedef struct {
    const void *data;
    size_t len;
} td_netlink_part_t;

/* psa_crypto_init. Safe to call more than once. */
bool td_netlink_crypto_init(void);

bool td_netlink_sha256(const td_netlink_part_t *parts, size_t count, uint8_t out[TD_NETLINK_H_LEN]);
bool td_netlink_sha512(const td_netlink_part_t *parts, size_t count, uint8_t out[TD_NETLINK_H512_LEN]);
bool td_netlink_hmac(const uint8_t *key, size_t key_len, const td_netlink_part_t *parts, size_t count,
                     uint8_t out[TD_NETLINK_H_LEN]);
/* HKDF-Expand-SHA256(prk, "touchdeck/net-v2/" || label, 32), one block. HKDF-Extract is td_netlink_hmac keyed with
 * the salt. */
bool td_netlink_derive(const uint8_t prk[TD_NETLINK_H_LEN], const char *label, uint8_t out[TD_NETLINK_KEY_LEN]);
/* AES-256-GCM. out receives len + 16 bytes, ciphertext then tag. */
bool td_netlink_seal(const uint8_t key[TD_NETLINK_KEY_LEN], const uint8_t nonce[TD_NETLINK_NONCE_LEN],
                     const uint8_t *aad, size_t aad_len, const uint8_t *in, size_t len, uint8_t *out);
/* in holds len bytes including the tag; out receives len - 16 bytes. */
bool td_netlink_open(const uint8_t key[TD_NETLINK_KEY_LEN], const uint8_t nonce[TD_NETLINK_NONCE_LEN],
                     const uint8_t *aad, size_t aad_len, const uint8_t *in, size_t len, uint8_t *out);

/* X25519 (RFC 7748). The secret is any 32 bytes; PSA clamps it on import. */
bool td_netlink_x25519_share(const uint8_t secret[TD_NETLINK_SHARE_LEN], uint8_t share[TD_NETLINK_SHARE_LEN]);
/* net-v2's share rule: bit 255 clear and u < 2^255 - 19. */
bool td_netlink_share_canonical(const uint8_t share[TD_NETLINK_SHARE_LEN]);
/* Z = X25519(secret, peer). False for a share that isn't canonical, one PSA refuses, or an all-zero Z: one failure. */
bool td_netlink_x25519(const uint8_t secret[TD_NETLINK_SHARE_LEN], const uint8_t peer[TD_NETLINK_SHARE_LEN],
                       uint8_t z[TD_NETLINK_SHARE_LEN]);

/* Constant time. */
bool td_netlink_equal(const uint8_t *a, const uint8_t *b, size_t len);
void td_netlink_wipe(void *p, size_t len);

#ifdef __cplusplus
}
#endif
