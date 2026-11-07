#pragma once

#include "td_netlink_crypto.h"
#include "td_netlink_record.h"
#include "td_netlink_srp.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The Device side of net-v2 (schema/protocol/net-v2.md), without any socket: pairing windows, SRP pairing, the X25519
 * session handshake, records, the stored Pairings, the pairing failure counter and lockout, and per-peer backoff. A
 * carrier (the TCP service, later BLE) feeds it whole items and does what each step returns. Time, randomness and
 * storage come from td_netlink_env_t, so the host tests drive it with the shared vectors.
 */

#define TD_NETLINK_PORT 47820
#define TD_NETLINK_VERSION 2
#define TD_NETLINK_HANDSHAKE_MIN 3
#define TD_NETLINK_HANDSHAKE_MAX 512
#define TD_NETLINK_ITEM_MAX (2 + TD_NETLINK_SEALED_MAX)
#define TD_NETLINK_PAIRINGS_MAX 4
#define TD_NETLINK_NAME_MAX 64
#define TD_NETLINK_DEVICE_ID_LEN 6
#define TD_NETLINK_COMPANION_ID_LEN 16
#define TD_NETLINK_OPEN_TAG_LEN 16
#define TD_NETLINK_PEER_LEN 16
#define TD_NETLINK_PEERS 8
/* Connections that wait for their first item while an owner exists, because one may take over an unproven owner. */
#define TD_NETLINK_WAITERS 3

#define TD_NETLINK_WINDOW_MS 120000u
#define TD_NETLINK_FIRST_ITEM_MS 2000u
#define TD_NETLINK_SETUP_MS 10000u
#define TD_NETLINK_ATTEMPT_MS 10000u
#define TD_NETLINK_HELLO_MS 5000u
#define TD_NETLINK_EXPIRY_MS 5000u
#define TD_NETLINK_PROOFS_MAX 3
#define TD_NETLINK_LOCK_AFTER 3
#define TD_NETLINK_LOCK_FIRST_MS 30000u
#define TD_NETLINK_LOCK_CAP_MS 900000u
#define TD_NETLINK_LOCK_SAVE_MS 60000u
#define TD_NETLINK_BACKOFF_FIRST_MS 1000u
#define TD_NETLINK_BACKOFF_CAP_MS 60000u

typedef enum {
    TD_NETLINK_PAIR_REQUEST = 0x10,
    TD_NETLINK_PAIR_OFFER = 0x11,
    TD_NETLINK_PAIR_START = 0x12,
    TD_NETLINK_PAIR_CHALLENGE = 0x13,
    TD_NETLINK_PAIR_PROOF = 0x14,
    TD_NETLINK_PAIR_ACCEPT = 0x15,
    TD_NETLINK_PAIR_COMMIT = 0x16,
    TD_NETLINK_PAIR_STORED = 0x17,
    TD_NETLINK_SESSION_OPEN = 0x20,
    TD_NETLINK_SESSION_CHALLENGE = 0x21,
    TD_NETLINK_SESSION_PROOF = 0x22,
    TD_NETLINK_SESSION_READY = 0x23,
    TD_NETLINK_ERROR = 0x7f,
} td_netlink_kind_t;

/* The handshake error reasons, and REASON_NONE for success. */
typedef enum {
    TD_NETLINK_REASON_NONE = 0,
    TD_NETLINK_BUSY = 1,
    TD_NETLINK_BAD_VERSION = 2,
    TD_NETLINK_ARGS = 3,
    TD_NETLINK_FULL = 4,
    TD_NETLINK_AUTH = 5,
    TD_NETLINK_EXPIRED = 6,
    TD_NETLINK_CANCELLED = 7,
    TD_NETLINK_STORAGE = 8,
    TD_NETLINK_CLOSED = 9,
} td_netlink_reason_t;

/* How a carrier connection ended. Only DEADLINE, AUTH, PROOF, MALFORMED and PEER_CLOSED count toward per-peer backoff,
 * and READY resets it. */
typedef enum {
    TD_NETLINK_END_NONE = 0,
    TD_NETLINK_END_READY,
    TD_NETLINK_END_DEADLINE,
    TD_NETLINK_END_AUTH,
    TD_NETLINK_END_PROOF,
    TD_NETLINK_END_MALFORMED,
    TD_NETLINK_END_PEER_CLOSED,
    TD_NETLINK_END_TAKEOVER,
    TD_NETLINK_END_BUSY,
    TD_NETLINK_END_USB_ONLINE,
    TD_NETLINK_END_CLOSED,
    TD_NETLINK_END_FULL,
    TD_NETLINK_END_CANCELLED,
    TD_NETLINK_END_STORAGE,
    TD_NETLINK_END_WINDOW_EXPIRED,
    TD_NETLINK_END_RADIO_LOST,
    TD_NETLINK_END_PAIRED,
    /* The Device ended it for its own reason, such as net.forget. */
    TD_NETLINK_END_LOCAL,
} td_netlink_end_t;

/* Why the carrier went away, when td_netlink_conn_close is not the end a step already decided. */
typedef enum {
    TD_NETLINK_GONE_DEVICE,
    /* TCP FIN or RST, or a BLE disconnect the peer sent (0x13, 0x14, 0x15). */
    TD_NETLINK_GONE_PEER,
    /* Any other BLE disconnect the Device didn't cause, such as a supervision timeout. */
    TD_NETLINK_GONE_RADIO,
} td_netlink_gone_t;

typedef struct {
    uint8_t companion_id[TD_NETLINK_COMPANION_ID_LEN];
    uint8_t name_len;
    char name[TD_NETLINK_NAME_MAX];
    uint8_t ltk[TD_NETLINK_KEY_LEN];
} td_netlink_pairing_t;

typedef struct {
    uint8_t count;
    td_netlink_pairing_t entries[TD_NETLINK_PAIRINGS_MAX];
} td_netlink_pairings_t;

/* What the core needs from the Device. Every callback runs on the caller's task. */
typedef struct {
    void (*rng)(void *ctx, uint8_t *out, size_t len);
    /* Monotonic milliseconds; wraps after 49 days. */
    uint32_t (*clock)(void *ctx);
    /* Persists the whole table atomically. True once it is durable. */
    bool (*save_pairings)(void *ctx, const td_netlink_pairings_t *pairings);
    /* Persists pair_failures and the pairing lockout's time left. True once it is durable. */
    bool (*save_failures)(void *ctx, uint32_t pair_failures, uint32_t lock_left_ms);
    /* True while the USB link is online. */
    bool (*usb_online)(void *ctx);
    void *ctx;
} td_netlink_env_t;

typedef struct {
    bool open;
    bool bound;
    /* Failed client proofs in this window. */
    uint8_t proofs;
    char code[TD_NETLINK_CODE_LEN];
    uint8_t window_id[16];
    uint32_t opened_ms;
    /* The bound connection's companion_name, for the pairing screen. */
    uint8_t name_len;
    char name[TD_NETLINK_NAME_MAX];
    /* Why the last window ended, for the connection that was bound to it. */
    td_netlink_reason_t ended;
    uint32_t generation;
} td_netlink_window_t;

typedef struct {
    bool used;
    uint8_t len;
    uint8_t key[TD_NETLINK_PEER_LEN];
    uint32_t failures;
    uint32_t until_ms;
    uint32_t last_ms;
} td_netlink_peer_t;

/* A carrier connection that started while an owner existed and waits for its first item. */
typedef struct {
    bool used;
    /* A byte of its first item has arrived, so a newer connection can't push it out. */
    bool started;
    uint8_t peer_len;
    uint8_t peer[TD_NETLINK_PEER_LEN];
    uint32_t since_ms;
} td_netlink_waiter_t;

typedef struct {
    uint8_t device_id[TD_NETLINK_DEVICE_ID_LEN];
    td_netlink_pairings_t pairings;
    td_netlink_window_t window;
    uint32_t pair_failures;
    bool locked;
    uint32_t lock_until_ms;
    uint32_t lock_saved_ms;
    td_netlink_peer_t peers[TD_NETLINK_PEERS];
    td_netlink_waiter_t waiters[TD_NETLINK_WAITERS];
    td_netlink_env_t env;
} td_netlink_deck_t;

typedef enum {
    TD_NETLINK_CONN_FIRST,
    TD_NETLINK_CONN_PAIR_CODE,
    TD_NETLINK_CONN_PAIR_PROOF,
    TD_NETLINK_CONN_PAIR_COMMIT,
    TD_NETLINK_CONN_SESSION_PROOF,
    TD_NETLINK_CONN_READY,
    TD_NETLINK_CONN_CLOSED,
} td_netlink_conn_state_t;

/* Pairing state that outlives one item. Erased when the attempt or the window ends. */
typedef struct {
    uint8_t request[2 + 2 + TD_NETLINK_COMPANION_ID_LEN + 1 + TD_NETLINK_NAME_MAX];
    uint16_t request_len;
    uint8_t offer[2 + 2 + 26];
    uint32_t window_generation;
    bool attempt_started;
    uint32_t attempt_ms;
    uint8_t transcript[TD_NETLINK_H_LEN];
    uint8_t client_proof[TD_NETLINK_H_LEN];
    uint8_t deck_key[TD_NETLINK_KEY_LEN];
    uint8_t wrap_key[TD_NETLINK_KEY_LEN];
    uint8_t commit_key[TD_NETLINK_KEY_LEN];
    uint8_t ltk[TD_NETLINK_KEY_LEN];
} td_netlink_pair_state_t;

typedef struct {
    uint8_t transcript[TD_NETLINK_H_LEN];
    uint8_t deck_proof[TD_NETLINK_H_LEN];
    uint8_t client_key[TD_NETLINK_KEY_LEN];
    uint8_t ready_key[TD_NETLINK_KEY_LEN];
    uint8_t c2d[TD_NETLINK_KEY_LEN];
    uint8_t d2c[TD_NETLINK_KEY_LEN];
} td_netlink_session_state_t;

typedef struct {
    td_netlink_conn_state_t state;
    uint32_t started_ms;
    td_netlink_end_t end;
    uint8_t peer[TD_NETLINK_PEER_LEN];
    uint8_t peer_len;
    union {
        td_netlink_pair_state_t pair;
        td_netlink_session_state_t session;
    } hs;
    uint8_t companion_id[TD_NETLINK_COMPANION_ID_LEN];
    uint8_t name_len;
    char name[TD_NETLINK_NAME_MAX];
    uint32_t ready_ms;
    uint32_t last_rx_ms;
    bool hello_seen;
    td_netlink_dir_t rx;
    td_netlink_dir_t tx;
} td_netlink_conn_t;

/* What the carrier does with one step's output. */
typedef enum {
    TD_NETLINK_NOTHING,
    TD_NETLINK_SEND,
    /* Send out, then end the carrier connection: TCP shuts its write side and drains for at most 100 ms. */
    TD_NETLINK_SEND_CLOSE,
    /* End the carrier connection at once with nothing sent. */
    TD_NETLINK_CLOSE,
} td_netlink_action_t;

typedef struct {
    uint8_t item[2 + TD_NETLINK_HANDSHAKE_MAX];
    uint16_t len;
} td_netlink_out_t;

/* Starts the Device state from what NVS held: the Pairings, pair_failures and the lockout's time left. */
void td_netlink_deck_init(td_netlink_deck_t *deck, const uint8_t device_id[TD_NETLINK_DEVICE_ID_LEN],
                          const td_netlink_pairings_t *stored, uint32_t pair_failures, uint32_t lock_left_ms,
                          const td_netlink_env_t *env);
/* A pairing window, opened only by a person at the Device or over USB. REASON_NONE when one is open afterwards (an open
 * window stays as it is), FULL with four Pairings, CLOSED during a lockout. */
td_netlink_reason_t td_netlink_window_open(td_netlink_deck_t *deck);
/* Cancel on the pairing screen. A bound connection gets `cancelled` on its next tick. */
void td_netlink_window_cancel(td_netlink_deck_t *deck);
/* Window expiry, lockout expiry, and the lockout's time left saved once a minute. */
void td_netlink_deck_tick(td_netlink_deck_t *deck);
/* Milliseconds left in a pairing lockout, 0 when there is none. */
uint32_t td_netlink_lock_left(const td_netlink_deck_t *deck);
/* Milliseconds left in the open window, 0 when none is open. */
uint32_t td_netlink_window_left(const td_netlink_deck_t *deck);
/* net.forget: cancels any window, erases every Pairing, and resets pair_failures and the lockout. False when a store
 * failed; the RAM state is cleared either way. */
bool td_netlink_forget(td_netlink_deck_t *deck);
/* Milliseconds left in this peer's backoff, 0 when there is none. */
uint32_t td_netlink_backoff_left(const td_netlink_deck_t *deck, const uint8_t *peer, size_t peer_len);
/* True when a whole handshake item body is a well-formed session.open whose open_tag matches a stored Pairing: the item
 * that may take over from an unproven owner and that skips per-peer backoff. */
bool td_netlink_names_stored(const td_netlink_deck_t *deck, const uint8_t *body, size_t len);
/* open_tag = the first 16 bytes of MAC(derive(LTK, "session/open"), "open" || client_share). */
bool td_netlink_open_tag(const uint8_t ltk[TD_NETLINK_KEY_LEN], const uint8_t share[TD_NETLINK_SHARE_LEN],
                         uint8_t tag[TD_NETLINK_OPEN_TAG_LEN]);
/* net-v2's companion_name rule: 1 to 64 bytes of UTF-8 with no control or format character and no Hangul filler. */
bool td_netlink_name_valid(const uint8_t *name, size_t len);

/* A new carrier connection from peer (a TCP source address or a BLE address, at most 16 bytes). */
void td_netlink_conn_init(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *peer, size_t peer_len);
/* Bounds for the next item's u16be length, checked before its body is read. */
void td_netlink_conn_limits(const td_netlink_conn_t *conn, uint16_t *min, uint16_t *max);
/* A length outside td_netlink_conn_limits: closes the connection as malformed. */
td_netlink_action_t td_netlink_conn_bad_length(td_netlink_conn_t *conn, td_netlink_deck_t *deck,
                                               td_netlink_out_t *out);
/* The first byte of an item arrived, which starts a pairing attempt's deadline. */
void td_netlink_conn_item_started(td_netlink_conn_t *conn, td_netlink_deck_t *deck);
/* One handshake item body (kind, version, fields) after its length prefix. */
td_netlink_action_t td_netlink_conn_item(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                         size_t len, td_netlink_out_t *out);
/* Deadlines, Cancel, window expiry, USB coming online, and session liveness. */
td_netlink_action_t td_netlink_conn_tick(td_netlink_conn_t *conn, td_netlink_deck_t *deck, td_netlink_out_t *out);
/* A session record after its length prefix. body receives len - 16 bytes. False closes the session. */
bool td_netlink_conn_record(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *sealed, size_t len,
                            uint8_t *body);
/* Seals one link frame body. wire receives 2 + len + 16 bytes. False closes the session. */
bool td_netlink_conn_send(td_netlink_conn_t *conn, const uint8_t *body, size_t len, uint8_t *wire);
/* The link saw `hello` on this session, so liveness now follows the last valid record. */
void td_netlink_conn_hello(td_netlink_conn_t *conn);
/* Proven: a session past client_proof. Unproven owners that aren't bound to a pairing window can be taken over. */
bool td_netlink_conn_proven(const td_netlink_conn_t *conn);
bool td_netlink_conn_bound(const td_netlink_conn_t *conn, const td_netlink_deck_t *deck);
/* A ready session with fewer than TD_NETLINK_COUNTER_MARGIN records left in either direction. The service ends it
 * between requests, and the Companion reconnects for fresh keys. */
bool td_netlink_conn_worn(const td_netlink_conn_t *conn);
/* Ends the connection as `end` unless a step already decided its end: the Device took it over, or closed it for its own
 * reason. */
void td_netlink_conn_end(td_netlink_conn_t *conn, td_netlink_deck_t *deck, td_netlink_end_t end);
/* The carrier connection is gone. Records the end for per-peer backoff and pair_failures, erases every key, and
 * destroys a window it was bound to. */
void td_netlink_conn_close(td_netlink_conn_t *conn, td_netlink_deck_t *deck, td_netlink_gone_t how);

/* ---- Admission: the owner and the connections waiting to take it over ----
 *
 * A carrier connection that starts with no owner becomes the owner. While the owner is proven or bound to a window,
 * every other one gets `busy` at once. While it is unproven, a new one waits in a slot for its first item, because a
 * session.open with a stored Pairing's open_tag takes over. With every slot taken, the newest connection pushes out the
 * oldest waiter that hasn't sent a byte, or else the oldest from a peer in backoff (it gets `busy`, which doesn't
 * count), so idle connections can't keep a paired Companion out. A waiter that misses the 2000 ms first-item deadline, or closes before its first item, counts toward
 * its peer's backoff. */
typedef enum {
    /* No owner: start a connection with td_netlink_conn_init. */
    TD_NETLINK_ADMIT_OWNER,
    /* Read its first item; *slot is its waiter slot. */
    TD_NETLINK_ADMIT_WAIT,
    /* Send td_netlink_busy_item and close. */
    TD_NETLINK_ADMIT_BUSY,
} td_netlink_admit_t;

/* A new carrier connection. owner is the owner's connection, NULL when there is none. *evicted is the slot of a waiter
 * to send `busy` and close first, or -1. */
td_netlink_admit_t td_netlink_admit(td_netlink_deck_t *deck, const td_netlink_conn_t *owner, const uint8_t *peer,
                                    size_t peer_len, int *slot, int *evicted);
/* The first byte of a waiter's first item arrived. */
void td_netlink_waiter_started(td_netlink_deck_t *deck, int slot);
/* A waiter's whole first item. TD_NETLINK_NOTHING: it takes over (or there is no owner any more), so end the owner as
 * TAKEOVER, start this connection with td_netlink_conn_init and feed it the item. TD_NETLINK_SEND_CLOSE: send out
 * (`busy`, or `auth` for a session.open whose tag matches no Pairing) and close. The slot is free afterwards. */
td_netlink_action_t td_netlink_waiter_item(td_netlink_deck_t *deck, const td_netlink_conn_t *owner, int slot,
                                           const uint8_t *body, size_t len, td_netlink_out_t *out);
/* Each poll, for each used slot: SEND_CLOSE with `expired` after 2000 ms without its first item (counted), or with
 * `busy` once the owner is proven or bound (not counted). The slot is free after SEND_CLOSE. */
td_netlink_action_t td_netlink_waiter_tick(td_netlink_deck_t *deck, const td_netlink_conn_t *owner, int slot,
                                           td_netlink_out_t *out);
/* The waiter's peer closed it (TCP FIN or RST) before its first item: counted. The slot is free afterwards. */
void td_netlink_waiter_gone(td_netlink_deck_t *deck, int slot);
/* The Device closed it for its own reason: not counted. */
void td_netlink_waiter_drop(td_netlink_deck_t *deck, int slot);

/* ---- Discovery privacy ----
 *
 * The TXT record of `_touchdeck._tcp`. Outside pairing mode it carries only `proto`, a random nonce `n` and four
 * 8-byte hints `h`: one per stored Pairing, MAC(derive(LTK, "discovery"), "hint" || nonce) cut to 8 bytes, and a
 * random filler per free slot. In pairing mode it adds the Device id, name and Pairing count. The carrier draws a new
 * nonce and fillers at boot, every TD_NETLINK_ADVERT_ROTATE_MS, and whenever the record's content changes. */
#define TD_NETLINK_ADVERT_NONCE_LEN 16
#define TD_NETLINK_HINT_LEN 8
#define TD_NETLINK_HINT_SLOTS TD_NETLINK_PAIRINGS_MAX
#define TD_NETLINK_ADVERT_ROTATE_MS 900000u
#define TD_NETLINK_ADVERT_NAME_MAX 48
#define TD_NETLINK_ADVERT_TXT_MAX 6

typedef struct {
    /* The first 4 bytes of the nonce in hex: the instance is "TouchDeck <label>" and the host "td-<label>". */
    char label[9];
    char instance[19];
    char host[12];
    char n[2 * TD_NETLINK_ADVERT_NONCE_LEN + 1];
    char h[2 * TD_NETLINK_HINT_SLOTS * TD_NETLINK_HINT_LEN + 1];
    /* Pairing mode: id, name and paired are set. */
    bool pairing;
    char id[2 * TD_NETLINK_DEVICE_ID_LEN + 1];
    char name[TD_NETLINK_ADVERT_NAME_MAX + 1];
    char paired[2];
} td_netlink_advert_t;

typedef struct {
    const char *key;
    const char *value;
} td_netlink_txt_t;

/* hint = the first 8 bytes of MAC(derive(LTK, "discovery"), "hint" || nonce). */
bool td_netlink_hint(const uint8_t ltk[TD_NETLINK_KEY_LEN], const uint8_t nonce[TD_NETLINK_ADVERT_NONCE_LEN],
                     uint8_t hint[TD_NETLINK_HINT_LEN]);
/* Pairing mode: a pairing window is open, so the record may name the Device. */
bool td_netlink_pairing_mode(const td_netlink_deck_t *deck);
/* Builds the record from the stored Pairings and the window. fillers holds 8 random bytes per slot; the stored
 * Pairings' slots are overwritten with their hints. name is the link-v2 Device name, used only in pairing mode, cut to
 * 48 bytes, and replaced with "TouchDeck" when empty or invalid. False when a hint couldn't be computed. */
bool td_netlink_advert_build(const td_netlink_deck_t *deck, const uint8_t nonce[TD_NETLINK_ADVERT_NONCE_LEN],
                             const uint8_t fillers[TD_NETLINK_HINT_SLOTS * TD_NETLINK_HINT_LEN], const char *name,
                             td_netlink_advert_t *out);
/* The record's keys and values, pointing into advert. Returns how many: 3, or 6 in pairing mode. */
size_t td_netlink_advert_txt(const td_netlink_advert_t *advert, td_netlink_txt_t txt[TD_NETLINK_ADVERT_TXT_MAX]);

/* A complete `busy` error item, for a connection the Device refuses before it has any state. */
extern const uint8_t td_netlink_busy_item[5];

#ifdef __cplusplus
}
#endif
