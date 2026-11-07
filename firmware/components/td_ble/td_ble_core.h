#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "key_sink.h"
#include "td_ble_hid.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The Bluetooth keyboard's logic, with no NimBLE or ESP-IDF in it: the opt-in setting, pairing and bonds, the one
 * advertising and backoff state machine, the per-connection state and the report queue. td_ble_nimble.c feeds it
 * NimBLE's events and does what it asks through td_ble_port_t; the host tests feed it the same events through a fake
 * port. The rules come from the crash-dump analysis in #275 (see docs/firmware.md, Bluetooth keyboard):
 *
 * - BLE is off until the person turns it on. The setting persists, and only a stored "on" starts the stack at boot.
 * - Advertising starts only on an event: the host syncing, a disconnect, advertising ending, a command, or the backoff
 *   timer after a start failed. Never while the host holds any connection object, which exists before NimBLE delivers
 *   the late CONNECT event. The accept list is written once after the stack starts and again only when the bonds
 *   change.
 * - Reconnecting to bonded hosts advertises at 20-30 ms for at most 30 s, then at 1-2.5 s. A failed start backs off
 *   1, 2, 4, 8 s and never retries in the same event.
 * - Every event is handled by its own connection handle. ENC_CHANGE before CONNECT is normal.
 * - A Security Request goes out only on a link that isn't encrypted yet. Connection parameters are requested at most
 *   once per connection, after encryption, and never again after a refusal.
 * - Pairing happens only inside a window the person opens, with at most TD_BLE_BONDS_MAX bonds.
 *
 * Not thread-safe: the caller serializes every call but td_ble_core_report. That one touches only the report queue,
 * which the port's lock guards, so the key sink may call it from any task without the caller's serialization.
 */

#define TD_BLE_BONDS_MAX 4
#define TD_BLE_CONNS_MAX 2
#define TD_BLE_CONN_NONE 0xFFFF
#define TD_BLE_PAIR_MS 120000u
#define TD_BLE_FAST_MS 30000u
#define TD_BLE_RETRY_MIN_MS 1000u
#define TD_BLE_RETRY_MAX_MS 8000u
#define TD_BLE_QUEUE_SIZE 32

typedef struct {
    uint8_t type;
    uint8_t val[6];
} td_ble_addr_t;

typedef enum {
    /* General discoverable, connectable by anyone, 30-50 ms, for the rest of the pairing window. */
    TD_BLE_ADV_PAIRING,
    /* Bonded hosts only (accept list), 20-30 ms. */
    TD_BLE_ADV_FAST,
    /* Bonded hosts only (accept list), 1-2.5 s. */
    TD_BLE_ADV_SLOW,
} td_ble_adv_t;

typedef struct {
    bool encrypted;
    bool bonded;
    /* The host's identity address. */
    td_ble_addr_t peer;
} td_ble_link_t;

/*
 * What the core asks of the radio and the store. Every function is called with the caller's serialization held,
 * except where it says otherwise. Return codes: 0 is success.
 */
typedef struct {
    void *ctx;
    /* The persisted setting. True only when it is stored and reads as on; missing or unreadable is off. */
    bool (*load_enabled)(void *ctx);
    int (*save_enabled)(void *ctx, bool on);
    /* Starts the controller and the host. The host reports sync later through td_ble_core_synced. */
    int (*stack_start)(void *ctx);
    /* Stops the host and the controller, and frees them. May drop the caller's serialization while it waits for the
     * host task, so events can run in between; they find the setting off. */
    void (*stack_stop)(void *ctx);
    uint32_t (*now_ms)(void *ctx);
    /* How many connection objects the host holds, including one whose CONNECT event hasn't arrived. */
    int (*conn_count)(void *ctx);
    bool (*link)(void *ctx, uint16_t conn, td_ble_link_t *out);
    /* The bonded hosts' identity addresses. */
    size_t (*bonds)(void *ctx, td_ble_addr_t out[TD_BLE_BONDS_MAX]);
    /* Deletes a bond and drops a link to that host. */
    int (*bond_delete)(void *ctx, const td_ble_addr_t *peer);
    /* Deletes a bond's stored keys only, keeping the link, for a bonded host that pairs again. */
    int (*keys_delete)(void *ctx, const td_ble_addr_t *peer);
    int (*accept_list)(void *ctx, const td_ble_addr_t *peers, size_t count);
    /* duration_ms 0 means until stopped. */
    int (*adv_start)(void *ctx, td_ble_adv_t mode, uint32_t duration_ms);
    void (*adv_stop)(void *ctx);
    int (*security)(void *ctx, uint16_t conn);
    int (*conn_params)(void *ctx, uint16_t conn);
    void (*terminate)(void *ctx, uint16_t conn);
    /* One one-shot timer: arms it to fire td_ble_core_timer after delay_ms, replacing an earlier arm. */
    void (*timer)(void *ctx, uint32_t delay_ms);
    void (*timer_cancel)(void *ctx);
    int (*notify)(void *ctx, uint16_t conn, td_ble_input_t input, const uint8_t *data, size_t len);
    /* Guard the report queue, which the key sink fills from other tasks. */
    void (*lock)(void *ctx);
    void (*unlock)(void *ctx);
} td_ble_port_t;

typedef struct {
    bool used;
    uint16_t handle;
    /* CONNECT arrived. */
    bool connected;
    bool encrypted;
    bool bonded;
    bool security_sent;
    bool params_sent;
    bool suspended;
    uint8_t protocol;
    uint8_t leds;
    bool notify[TD_BLE_IN_COUNT];
    td_ble_addr_t peer;
} td_ble_conn_t;

typedef struct {
    key_report_t entries[TD_BLE_QUEUE_SIZE];
    /* The latest keyboard and consumer reports, sent first after a resync. */
    key_report_t current[2];
    size_t head;
    size_t count;
    /* Bits 0 and 1: the keyboard and consumer snapshots still to send. */
    unsigned sync;
    bool online;
} td_ble_queue_t;

/* Counters for `ble state`, `radio state` and the host tests. */
typedef struct {
    uint32_t adv_starts;
    uint32_t adv_failures;
    uint32_t accept_writes;
    uint32_t security_requests;
    uint32_t param_requests;
    uint32_t stack_starts;
} td_ble_stats_t;

typedef struct {
    const td_ble_port_t *port;
    bool enabled;
    bool running;
    bool start_failed;
    bool synced;
    bool advertising;
    td_ble_adv_t adv_mode;
    bool accept_dirty;
    bool pairing;
    uint32_t pair_until;
    /* A reconnect episode starts when advertising to bonds starts after a sync, a disconnect or a bond change. */
    bool episode;
    uint32_t fast_until;
    bool retry_armed;
    uint32_t retry_at;
    uint32_t retry_ms;
    /* A notification the host couldn't queue: drain again from the current state at drain_at. */
    bool drain_due;
    uint32_t drain_at;
    td_ble_conn_t conns[TD_BLE_CONNS_MAX];
    td_ble_addr_t bonds[TD_BLE_BONDS_MAX];
    size_t bond_count;
    td_ble_stats_t stats;
    td_ble_queue_t queue;
} td_ble_core_t;

typedef enum {
    TD_BLE_OK,
    TD_BLE_ERR_OFF,
    TD_BLE_ERR_STARTING,
    TD_BLE_ERR_FULL,
    TD_BLE_ERR_STORAGE,
    TD_BLE_ERR_RADIO,
} td_ble_result_t;

typedef struct {
    bool enabled;
    bool running;
    bool start_failed;
    bool synced;
    bool pairing;
    uint32_t pairing_ms;
    bool advertising;
    td_ble_adv_t adv_mode;
    size_t bond_count;
    td_ble_addr_t bonds[TD_BLE_BONDS_MAX];
    /* Per bond: a link to it is up, encrypted and bonded. */
    bool bond_connected[TD_BLE_BONDS_MAX];
    td_ble_stats_t stats;
} td_ble_status_t;

void td_ble_core_init(td_ble_core_t *core, const td_ble_port_t *port);

/* At boot: starts the stack only when the stored setting is on. */
void td_ble_core_boot(td_ble_core_t *core);

/* Host events. */
void td_ble_core_synced(td_ble_core_t *core);
void td_ble_core_host_reset(td_ble_core_t *core);
void td_ble_core_connect(td_ble_core_t *core, uint16_t conn, int status);
void td_ble_core_disconnect(td_ble_core_t *core, uint16_t conn);
void td_ble_core_enc_change(td_ble_core_t *core, uint16_t conn, int status);
/* Advertising ended on its own. `expired` when its duration ran out; any other end counts as a failed start and
 * backs off, so advertising that keeps ending at once can't turn into a loop. */
void td_ble_core_adv_complete(td_ble_core_t *core, bool expired);
void td_ble_core_subscribe(td_ble_core_t *core, uint16_t conn, td_ble_input_t input, bool notify);
void td_ble_core_timer(td_ble_core_t *core);
/* A bonded host wants to pair again: true lets it, inside the pairing window only, after deleting its old bond. */
bool td_ble_core_repeat_pairing(td_ble_core_t *core, uint16_t conn);

/* GATT access: the HID characteristics answer only an encrypted link with a bonded host. */
bool td_ble_core_may_access(const td_ble_core_t *core, uint16_t conn);
uint8_t td_ble_core_protocol(const td_ble_core_t *core, uint16_t conn);
uint8_t td_ble_core_leds(const td_ble_core_t *core, uint16_t conn);
/* Writes. False refuses the value. */
bool td_ble_core_set_protocol(td_ble_core_t *core, uint16_t conn, uint8_t mode);
bool td_ble_core_control_point(td_ble_core_t *core, uint16_t conn, uint8_t command);
void td_ble_core_set_leds(td_ble_core_t *core, uint16_t conn, uint8_t leds);
/* The current report for a read of an input characteristic. */
size_t td_ble_core_current(td_ble_core_t *core, td_ble_input_t input, uint8_t out[TD_BLE_KEYBOARD_BYTES]);

/* Commands. */
td_ble_result_t td_ble_core_set_enabled(td_ble_core_t *core, bool on);
td_ble_result_t td_ble_core_pair(td_ble_core_t *core, bool open);
/* NULL forgets every bond. *forgotten counts the bonds deleted. */
td_ble_result_t td_ble_core_forget(td_ble_core_t *core, const td_ble_addr_t *peer, size_t *forgotten);
void td_ble_core_status(const td_ble_core_t *core, td_ble_status_t *out);

/* The key sink. Any task. True when the queue has something to send, so the caller schedules td_ble_core_drain. */
bool td_ble_core_report(td_ble_core_t *core, const key_report_t *report);
/* Sends what the queue holds to the bonded, subscribed host. */
void td_ble_core_drain(td_ble_core_t *core);

#ifdef __cplusplus
}
#endif
