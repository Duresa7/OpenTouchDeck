/*
 * The Bluetooth keyboard's logic on the host: td_ble_core.c, td_ble_msgs.c and td_ble_hid.c run unchanged against a
 * fake port that plays NimBLE, the controller and NVS. The tests replay the event orders from the crash analysis in
 * #275 and check the rules it set: BLE stays off unless the stored setting is on; advertising starts only from events
 * and never while a connection object exists; the accept list is written only when the bonds change; a failed start
 * backs off 1, 2, 4, 8 s; reconnect advertising runs fast for 30 s, then slow; a Security Request only on a link that
 * isn't encrypted; one connection-parameter request after encryption; every event by its own handle. And the pairing
 * rules from the #306 security review: a passkey on screen is the only way to pair, it leaves the screen whenever
 * pairing stops, Just Works and legacy keys are refused, and the resolving list follows the bond store.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "cJSON.h"
#include "td_ble_core.h"
#include "td_ble_service.h"

static int g_failures;

#define CHECK(cond, ...)                                                                                               \
    do {                                                                                                               \
        if (!(cond)) {                                                                                                 \
            printf("FAIL %s:%d: ", __FILE__, __LINE__);                                                                \
            printf(__VA_ARGS__);                                                                                       \
            printf("\n");                                                                                              \
            g_failures++;                                                                                              \
        }                                                                                                              \
    } while (0)

/* ---- The fake radio, host and store. ---- */

#define FAKE_CONNS 4

typedef struct {
    bool used;
    uint16_t handle;
    td_ble_link_t link;
} fake_conn_t;

typedef struct {
    /* NVS: -1 when the key was never written. */
    int stored;
    int save_rc;
    bool stack_running;
    int stack_start_rc;
    int stack_starts;
    int stack_stops;
    uint32_t now;
    fake_conn_t conns[FAKE_CONNS];
    td_ble_addr_t bonds[TD_BLE_BONDS_MAX];
    size_t bond_count;
    int bond_deletes;
    int keys_deletes;
    int accept_writes;
    size_t accept_count;
    /* The controller's resolving list, as the last write left it. */
    int resolving_rc;
    int resolving_writes;
    size_t resolving_count;
    td_ble_addr_t resolving[TD_BLE_BONDS_MAX];
    /* The advertising starts made before the last resolving-list write. */
    int adv_calls_at_resolving;
    /* The hardware random numbers, in order, then 0. */
    uint32_t randoms[4];
    int random_next;
    /* The screen. */
    bool passkey_on;
    uint32_t passkey_code;
    int passkey_shows;
    int passkey_hides;
    /* Advertising. */
    int adv_rc;
    int adv_calls;
    bool advertising;
    td_ble_adv_t adv_mode;
    uint32_t adv_duration;
    int adv_stops;
    int security_calls;
    int param_calls;
    int terminates;
    uint16_t last_terminated;
    /* The timer. */
    bool timer_armed;
    uint32_t timer_at;
    /* Notifications sent. */
    int notifies;
    td_ble_input_t last_input;
    uint8_t last_data[8];
    size_t last_len;
    int notify_rc;
} fake_t;

static fake_t F;
static td_ble_port_t PORT;
static td_ble_core_t CORE;

static bool fake_load_enabled(void *ctx)
{
    (void)ctx;
    return F.stored == 1;
}

static int fake_save_enabled(void *ctx, bool on)
{
    (void)ctx;
    if (F.save_rc == 0) {
        F.stored = on ? 1 : 0;
    }
    return F.save_rc;
}

static int fake_stack_start(void *ctx)
{
    (void)ctx;
    F.stack_starts++;
    if (F.stack_start_rc == 0) {
        F.stack_running = true;
    }
    return F.stack_start_rc;
}

static void fake_stack_stop(void *ctx)
{
    (void)ctx;
    F.stack_stops++;
    F.stack_running = false;
    F.advertising = false;
    memset(F.conns, 0, sizeof F.conns);
}

static uint32_t fake_now(void *ctx)
{
    (void)ctx;
    return F.now;
}

static int fake_conn_count(void *ctx)
{
    (void)ctx;
    int n = 0;
    for (int i = 0; i < FAKE_CONNS; i++) {
        n += F.conns[i].used;
    }
    return n;
}

static fake_conn_t *fake_conn(uint16_t handle)
{
    for (int i = 0; i < FAKE_CONNS; i++) {
        if (F.conns[i].used && F.conns[i].handle == handle) {
            return &F.conns[i];
        }
    }
    return NULL;
}

static bool fake_link(void *ctx, uint16_t conn, td_ble_link_t *out)
{
    (void)ctx;
    const fake_conn_t *c = fake_conn(conn);
    if (!c) {
        return false;
    }
    *out = c->link;
    return true;
}

static size_t fake_bonds(void *ctx, td_ble_addr_t out[TD_BLE_BONDS_MAX])
{
    (void)ctx;
    memcpy(out, F.bonds, F.bond_count * sizeof F.bonds[0]);
    return F.bond_count;
}

static bool addr_same(const td_ble_addr_t *a, const td_ble_addr_t *b)
{
    return a->type == b->type && !memcmp(a->val, b->val, sizeof a->val);
}

static int remove_bond(const td_ble_addr_t *peer)
{
    for (size_t i = 0; i < F.bond_count; i++) {
        if (addr_same(&F.bonds[i], peer)) {
            memmove(&F.bonds[i], &F.bonds[i + 1], (F.bond_count - i - 1) * sizeof F.bonds[0]);
            F.bond_count--;
            return 0;
        }
    }
    return -1;
}

static int fake_bond_delete(void *ctx, const td_ble_addr_t *peer)
{
    (void)ctx;
    F.bond_deletes++;
    return remove_bond(peer);
}

static int fake_keys_delete(void *ctx, const td_ble_addr_t *peer)
{
    (void)ctx;
    F.keys_deletes++;
    return remove_bond(peer);
}

static int fake_accept_list(void *ctx, const td_ble_addr_t *peers, size_t count)
{
    (void)ctx;
    (void)peers;
    /* The controller refuses a write while advertising, as a real one does. */
    if (F.advertising) {
        return 12;
    }
    F.accept_writes++;
    F.accept_count = count;
    return 0;
}

static int fake_resolving_list(void *ctx, const td_ble_addr_t *peers, size_t count)
{
    (void)ctx;
    /* The controller refuses changes to the resolving list while advertising or connected. */
    if (F.advertising || fake_conn_count(NULL) > 0) {
        return 12;
    }
    if (F.resolving_rc != 0) {
        return F.resolving_rc;
    }
    F.resolving_writes++;
    F.resolving_count = count;
    memcpy(F.resolving, peers, count * sizeof peers[0]);
    F.adv_calls_at_resolving = F.adv_calls;
    return 0;
}

static uint32_t fake_random(void *ctx)
{
    (void)ctx;
    return F.random_next < 4 ? F.randoms[F.random_next++] : 0;
}

static void fake_passkey_show(void *ctx, bool show, uint32_t passkey)
{
    (void)ctx;
    F.passkey_on = show;
    if (show) {
        F.passkey_shows++;
        F.passkey_code = passkey;
    } else {
        F.passkey_hides++;
    }
}

static int fake_adv_start(void *ctx, td_ble_adv_t mode, uint32_t duration_ms)
{
    (void)ctx;
    F.adv_calls++;
    /* NimBLE refuses advertising while its one connection object exists (ble_gap_adv_validate, BLE_HS_ENOMEM). */
    if (fake_conn_count(NULL) > 0) {
        return 6;
    }
    if (F.adv_rc != 0 || F.advertising) {
        return F.adv_rc ? F.adv_rc : 2;
    }
    F.advertising = true;
    F.adv_mode = mode;
    F.adv_duration = duration_ms;
    return 0;
}

static void fake_adv_stop(void *ctx)
{
    (void)ctx;
    F.adv_stops++;
    F.advertising = false;
}

static int fake_security(void *ctx, uint16_t conn)
{
    (void)ctx;
    (void)conn;
    F.security_calls++;
    return 0;
}

static int fake_conn_params(void *ctx, uint16_t conn)
{
    (void)ctx;
    (void)conn;
    F.param_calls++;
    return 0;
}

static void fake_terminate(void *ctx, uint16_t conn)
{
    (void)ctx;
    F.terminates++;
    F.last_terminated = conn;
}

static void fake_timer(void *ctx, uint32_t delay_ms)
{
    (void)ctx;
    F.timer_armed = true;
    F.timer_at = F.now + delay_ms;
}

static void fake_timer_cancel(void *ctx)
{
    (void)ctx;
    F.timer_armed = false;
}

static int fake_notify(void *ctx, uint16_t conn, td_ble_input_t input, const uint8_t *data, size_t len)
{
    (void)ctx;
    (void)conn;
    if (F.notify_rc != 0) {
        return F.notify_rc;
    }
    F.notifies++;
    F.last_input = input;
    F.last_len = len;
    memcpy(F.last_data, data, len);
    return 0;
}

static void fake_lock(void *ctx)
{
    (void)ctx;
}

static void fake_unlock(void *ctx)
{
    (void)ctx;
}

/* The serialized service td_ble_msgs.c calls; on the Device td_ble_nimble.c takes its lock around each. */
td_ble_result_t td_ble_set_enabled(bool on)
{
    return td_ble_core_set_enabled(&CORE, on);
}

td_ble_result_t td_ble_pair(bool open)
{
    return td_ble_core_pair(&CORE, open);
}

td_ble_result_t td_ble_forget(const td_ble_addr_t *peer, size_t *forgotten)
{
    return td_ble_core_forget(&CORE, peer, forgotten);
}

void td_ble_status(td_ble_status_t *out)
{
    td_ble_core_status(&CORE, out);
}

/* ---- Helpers that play the radio. ---- */

static const td_ble_addr_t HOST_A = { 0, { 0x01, 0x02, 0x03, 0x04, 0x05, 0x06 } };
static const td_ble_addr_t HOST_B = { 1, { 0x11, 0x12, 0x13, 0x14, 0x15, 0xC6 } };
static const td_ble_addr_t STRANGER = { 1, { 0x21, 0x22, 0x23, 0x24, 0x25, 0xE6 } };

static void reset(int stored)
{
    memset(&F, 0, sizeof F);
    F.stored = stored;
    F.now = 1000;
    PORT = (td_ble_port_t){
        .load_enabled = fake_load_enabled,
        .save_enabled = fake_save_enabled,
        .stack_start = fake_stack_start,
        .stack_stop = fake_stack_stop,
        .now_ms = fake_now,
        .conn_count = fake_conn_count,
        .link = fake_link,
        .bonds = fake_bonds,
        .bond_delete = fake_bond_delete,
        .keys_delete = fake_keys_delete,
        .accept_list = fake_accept_list,
        .resolving_list = fake_resolving_list,
        .adv_start = fake_adv_start,
        .adv_stop = fake_adv_stop,
        .security = fake_security,
        .conn_params = fake_conn_params,
        .terminate = fake_terminate,
        .timer = fake_timer,
        .timer_cancel = fake_timer_cancel,
        .notify = fake_notify,
        .random = fake_random,
        .passkey_show = fake_passkey_show,
        .lock = fake_lock,
        .unlock = fake_unlock,
    };
    td_ble_core_init(&CORE, &PORT);
}

static void add_bond(const td_ble_addr_t *peer)
{
    F.bonds[F.bond_count++] = *peer;
}

/* LE Connection Complete: the controller made the connection and NimBLE holds its object, but the CONNECT event comes
 * later, after Read Remote Version and Features. Advertising stopped with it. */
static void link_up(uint16_t handle, const td_ble_addr_t *peer)
{
    for (int i = 0; i < FAKE_CONNS; i++) {
        if (!F.conns[i].used) {
            F.conns[i] = (fake_conn_t){ .used = true, .handle = handle, .link = { .peer = *peer } };
            break;
        }
    }
    F.advertising = false;
}

/* The host encrypted the link with keys of the given kind: with a stored key, or at the end of pairing, which stores a
 * new bond. */
static void encrypt_with(uint16_t handle, bool new_bond, bool authenticated, bool secure_connections, uint8_t key_size)
{
    fake_conn_t *c = fake_conn(handle);
    c->link.encrypted = true;
    c->link.bonded = true;
    c->link.authenticated = authenticated;
    c->link.secure_connections = secure_connections;
    c->link.key_size = key_size;
    if (new_bond) {
        add_bond(&c->link.peer);
    }
    td_ble_core_enc_change(&CORE, handle, 0);
}

/* Keys from a passkey pairing: authenticated LE Secure Connections keys. */
static void encrypt(uint16_t handle, bool new_bond)
{
    encrypt_with(handle, new_bond, true, true, TD_BLE_KEY_SIZE);
}

/* A whole passkey pairing on a link that is up: the Security Manager asks to display, the host types the code, the
 * pairing completes and the link is encrypted with the new bond. */
static void pair_with_passkey(uint16_t handle)
{
    uint32_t code = 0;
    CHECK(td_ble_core_passkey_action(&CORE, handle, TD_BLE_IO_DISPLAY, &code), "passkey refused on %u", handle);
    CHECK(F.passkey_on && F.passkey_code == code, "passkey %06u not on screen", (unsigned)code);
    td_ble_core_pairing_complete(&CORE, handle, 0);
    encrypt(handle, true);
}

static void link_down(uint16_t handle)
{
    fake_conn_t *c = fake_conn(handle);
    if (c) {
        c->used = false;
    }
    td_ble_core_disconnect(&CORE, handle);
}

/* Moves the clock and fires the timer when it is due, as the host task would. */
static void advance(uint32_t ms)
{
    const uint32_t end = F.now + ms;
    while (F.timer_armed && (int32_t)(F.timer_at - end) <= 0) {
        F.now = F.timer_at;
        F.timer_armed = false;
        td_ble_core_timer(&CORE);
    }
    F.now = end;
}

/* The advertising set's duration ran out, which NimBLE reports as ADV_COMPLETE with BLE_HS_ETIMEOUT. */
static void adv_expires(void)
{
    F.now += F.adv_duration;
    F.advertising = false;
    td_ble_core_adv_complete(&CORE, true);
}

static void boot_synced(void)
{
    td_ble_core_boot(&CORE);
    td_ble_core_synced(&CORE);
}

static cJSON *message(const char *json)
{
    cJSON *request = cJSON_Parse(json);
    cJSON *reply = cJSON_CreateObject();
    cJSON_AddBoolToObject(reply, "ok", true);
    td_ble_message(request, reply);
    cJSON_Delete(request);
    return reply;
}

static const char *reason(const cJSON *reply)
{
    const char *r = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, "reason"));
    return r ? r : "";
}

static bool ok(const cJSON *reply)
{
    return cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "ok"));
}

/* ---- Tests. ---- */

static void test_off_unless_stored_on(void)
{
    /* A fresh Device: nothing stored. */
    reset(-1);
    td_ble_core_boot(&CORE);
    CHECK(F.stack_starts == 0, "a fresh Device started BLE");
    CHECK(F.adv_calls == 0 && F.accept_writes == 0 && !F.timer_armed, "a fresh Device touched the radio");
    td_ble_status_t status;
    td_ble_core_status(&CORE, &status);
    CHECK(!status.enabled && !status.running, "a fresh Device reports BLE on");

    /* Stored off. */
    reset(0);
    td_ble_core_boot(&CORE);
    CHECK(F.stack_starts == 0, "a stored off started BLE");

    /* Commands other than ble.set leave it off and don't store anything. */
    cJSON *reply = message("{\"m\":\"ble.pair\"}");
    CHECK(!ok(reply) && !strcmp(reason(reply), "off"), "ble.pair while off: %s", reason(reply));
    cJSON_Delete(reply);
    reply = message("{\"m\":\"ble.forget\"}");
    CHECK(!ok(reply) && !strcmp(reason(reply), "off"), "ble.forget while off: %s", reason(reply));
    cJSON_Delete(reply);
    reply = message("{\"m\":\"ble.state\"}");
    CHECK(ok(reply), "ble.state failed");
    CHECK(cJSON_IsFalse(cJSON_GetObjectItemCaseSensitive(reply, "enabled")), "ble.state says enabled");
    CHECK(!strcmp(cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, "state")), "off"), "state not off");
    CHECK(!strcmp(cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, "advertising")), "off"),
          "advertising not off");
    CHECK(cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(reply, "bonds_max")) == TD_BLE_BONDS_MAX, "bonds_max");
    CHECK(cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(reply, "battery")) == TD_BLE_BATTERY_LEVEL, "battery");
    cJSON_Delete(reply);
    CHECK(F.stack_starts == 0 && F.stored == 0, "a command other than ble.set started BLE or changed the setting");

    /* Turning it on stores the setting first, then starts the stack. */
    reply = message("{\"m\":\"ble.set\",\"enabled\":true}");
    CHECK(ok(reply), "ble.set on failed: %s", reason(reply));
    cJSON_Delete(reply);
    CHECK(F.stored == 1 && F.stack_starts == 1, "ble.set on: stored %d, starts %d", F.stored, F.stack_starts);

    /* A reboot with the stored on starts it again. */
    const int stored = F.stored;
    reset(stored);
    td_ble_core_boot(&CORE);
    CHECK(F.stack_starts == 1, "a stored on did not start BLE at boot");

    /* A setting that can't be saved leaves BLE as it was. */
    reset(0);
    td_ble_core_boot(&CORE);
    F.save_rc = 1;
    reply = message("{\"m\":\"ble.set\",\"enabled\":true}");
    CHECK(!ok(reply) && !strcmp(reason(reply), "storage"), "unsaved setting: %s", reason(reply));
    cJSON_Delete(reply);
    CHECK(F.stack_starts == 0, "BLE started without its setting saved");

    /* Off stops the stack and stores off. */
    reset(1);
    boot_synced();
    reply = message("{\"m\":\"ble.set\",\"enabled\":false}");
    CHECK(ok(reply), "ble.set off failed");
    cJSON_Delete(reply);
    CHECK(F.stored == 0 && F.stack_stops == 1 && !F.timer_armed, "ble.set off: stored %d stops %d", F.stored,
          F.stack_stops);
}

static void test_no_bonds_no_advertising(void)
{
    reset(1);
    boot_synced();
    CHECK(F.adv_calls == 0, "advertised with no bonds and no pairing window");
    CHECK(F.accept_writes == 0, "wrote the accept list with no bonds");
    advance(60000);
    CHECK(F.adv_calls == 0, "advertising started from the clock");
}

/* The reboot from the crash analysis: the bonded host reconnects to fast advertising, the controller reports
 * Connection Complete, the host encrypts with its stored key, and NimBLE delivers CONNECT about 100 ms later. */
static void test_reboot_reconnect_replay(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    CHECK(F.accept_writes == 1 && F.accept_count == 1, "accept list after sync: %d writes", F.accept_writes);
    CHECK(F.adv_calls == 1 && F.advertising && F.adv_mode == TD_BLE_ADV_FAST, "no fast advertising after sync");
    CHECK(F.adv_duration == TD_BLE_FAST_MS, "fast advertising for %u ms", (unsigned)F.adv_duration);

    link_up(7, &HOST_A);
    /* Nothing reaches the core for Connection Complete. Whatever runs in the gap must not advertise. */
    advance(50);
    td_ble_core_timer(&CORE);
    encrypt(7, false);
    advance(50);
    td_ble_core_connect(&CORE, 7, 0);

    CHECK(F.adv_calls == 1, "advertising retried %d times around the reconnect", F.adv_calls - 1);
    CHECK(CORE.stats.adv_failures == 0, "%u failed advertising starts", (unsigned)CORE.stats.adv_failures);
    CHECK(F.accept_writes == 1, "accept list written %d times", F.accept_writes);
    CHECK(F.security_calls == 0, "a Security Request on a link the host already encrypted");
    CHECK(F.param_calls == 1, "%d connection-parameter requests", F.param_calls);
    CHECK(F.terminates == 0, "the bonded host was dropped");

    /* Re-encryption and stray timer events change nothing. */
    td_ble_core_enc_change(&CORE, 7, 0);
    advance(120000);
    CHECK(F.param_calls == 1 && F.adv_calls == 1, "params %d, adv %d after the link settled", F.param_calls,
          F.adv_calls);

    /* The host subscribes; a key goes out with its 8 bytes and no Report ID. */
    td_ble_core_subscribe(&CORE, 7, TD_BLE_IN_KEYBOARD, true);
    td_ble_core_subscribe(&CORE, 7, TD_BLE_IN_CONSUMER, true);
    td_ble_core_drain(&CORE);
    const int resync_notifies = F.notifies;
    CHECK(resync_notifies == 2, "the resync sent %d reports, want the keyboard and consumer state", resync_notifies);
    key_report_t a = key_report_kbd(0, 0x04);
    CHECK(td_ble_core_report(&CORE, &a), "a report while online did not ask for a drain");
    td_ble_core_drain(&CORE);
    CHECK(F.notifies == resync_notifies + 1 && F.last_input == TD_BLE_IN_KEYBOARD && F.last_len == 8 &&
              F.last_data[2] == 0x04,
          "keyboard notify: %d sent, input %d, len %u", F.notifies, F.last_input, (unsigned)F.last_len);
    key_report_t vol = key_report_con(0x00E9);
    td_ble_core_report(&CORE, &vol);
    td_ble_core_drain(&CORE);
    CHECK(F.last_input == TD_BLE_IN_CONSUMER && F.last_len == 2 && F.last_data[0] == 0xE9, "consumer notify");

    /* The host goes away: a new episode advertises fast again, and the accept list is not rewritten. */
    link_down(7);
    CHECK(F.adv_calls == 2 && F.advertising && F.adv_mode == TD_BLE_ADV_FAST, "no fast advertising after disconnect");
    CHECK(F.accept_writes == 1, "accept list rewritten after a disconnect");
    CHECK(!td_ble_core_report(&CORE, &a), "a report while offline asked for a drain");
}

/* The host connects as fast advertising runs out: ADV_COMPLETE arrives while the connection object exists and before
 * CONNECT. Nothing may advertise, and nothing may count as a failed start. */
static void test_connect_races_the_end_of_advertising(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    link_up(8, &HOST_A);
    F.now += TD_BLE_FAST_MS;
    td_ble_core_adv_complete(&CORE, true);
    advance(5000);
    CHECK(F.adv_calls == 1, "advertising started %d times with a connection object up", F.adv_calls - 1);
    CHECK(CORE.stats.adv_failures == 0 && !F.timer_armed, "a refused start backed off: failures %u",
          (unsigned)CORE.stats.adv_failures);
    encrypt(8, false);
    td_ble_core_connect(&CORE, 8, 0);
    CHECK(F.adv_calls == 1 && F.accept_writes == 1 && F.security_calls == 0 && F.param_calls == 1,
          "after the race: adv %d writes %d security %d params %d", F.adv_calls, F.accept_writes, F.security_calls,
          F.param_calls);
    /* It goes away again: a new episode, fast. */
    link_down(8);
    CHECK(F.adv_calls == 2 && F.adv_mode == TD_BLE_ADV_FAST, "no fast advertising after the race");
}

static void test_fast_then_slow(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    CHECK(F.adv_mode == TD_BLE_ADV_FAST, "first phase not fast");
    adv_expires();
    CHECK(F.adv_calls == 2 && F.advertising && F.adv_mode == TD_BLE_ADV_SLOW && F.adv_duration == 0,
          "after 30 s: calls %d mode %d duration %u", F.adv_calls, F.adv_mode, (unsigned)F.adv_duration);
    advance(600000);
    CHECK(F.adv_calls == 2, "slow advertising restarted from the clock");
    CHECK(F.accept_writes == 1, "accept list rewritten between phases");
}

static void test_failed_start_backs_off(void)
{
    reset(1);
    add_bond(&HOST_A);
    F.adv_rc = 3;
    boot_synced();
    CHECK(F.adv_calls == 1, "%d starts in the sync event", F.adv_calls);
    const uint32_t want[] = { 1000, 2000, 4000, 8000, 8000 };
    for (size_t i = 0; i < sizeof want / sizeof want[0]; i++) {
        CHECK(F.timer_armed && F.timer_at - F.now == want[i], "backoff %u: timer in %u ms, want %u", (unsigned)i,
              (unsigned)(F.timer_at - F.now), (unsigned)want[i]);
        const int calls = F.adv_calls;
        /* An event in between does not retry early. */
        td_ble_core_timer(&CORE);
        CHECK(F.adv_calls == calls, "retried before the backoff ran out");
        advance(want[i]);
        CHECK(F.adv_calls == calls + 1, "backoff %u: %d starts, want one", (unsigned)i, F.adv_calls - calls);
    }
    /* Advertising that ends at once, without running out, counts as a failure too. */
    F.adv_rc = 0;
    advance(8000);
    CHECK(F.advertising, "advertising did not start once the radio took it");
    F.advertising = false;
    td_ble_core_adv_complete(&CORE, false);
    CHECK(!F.advertising && F.timer_armed && F.timer_at - F.now == 1000,
          "an early ADV_COMPLETE did not back off 1 s");
}

static void test_pairing_window(void)
{
    reset(1);
    boot_synced();
    cJSON *reply = message("{\"m\":\"ble.pair\",\"open\":true}");
    CHECK(ok(reply), "ble.pair failed: %s", reason(reply));
    CHECK(cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "pairing")), "pairing not reported");
    CHECK(cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(reply, "pairing_ms")) == TD_BLE_PAIR_MS, "pairing_ms");
    CHECK(!strcmp(cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, "advertising")), "pairing"),
          "advertising not pairing");
    cJSON_Delete(reply);
    CHECK(F.advertising && F.adv_mode == TD_BLE_ADV_PAIRING && F.adv_duration == TD_BLE_PAIR_MS,
          "pairing advertising %d mode %d for %u", F.advertising, F.adv_mode, (unsigned)F.adv_duration);

    /* A stranger connects inside the window: one Security Request, pairing, a new bond. */
    link_up(3, &STRANGER);
    td_ble_core_connect(&CORE, 3, 0);
    CHECK(F.security_calls == 1, "%d Security Requests to a host that isn't encrypted", F.security_calls);
    CHECK(F.param_calls == 0, "connection parameters before encryption");
    pair_with_passkey(3);
    CHECK(!F.passkey_on, "the passkey stayed on screen after pairing");
    CHECK(F.param_calls == 1, "%d parameter requests after pairing", F.param_calls);
    td_ble_status_t status;
    td_ble_core_status(&CORE, &status);
    CHECK(!status.pairing && status.bond_count == 1 && status.bond_connected[0], "window still open after a bond");

    /* It disconnects: reconnect advertising for the new bond, with a fresh accept list. */
    link_down(3);
    CHECK(F.advertising && F.adv_mode == TD_BLE_ADV_FAST && F.accept_writes == 1 && F.accept_count == 1,
          "after the bond: advertising %d mode %d writes %d", F.advertising, F.adv_mode, F.accept_writes);
}

/* The Security Manager asks to display a passkey: six random digits, equally likely, on screen until pairing ends. */
static void test_passkey_flow(void)
{
    reset(1);
    boot_synced();
    td_ble_core_pair(&CORE, true);
    link_up(3, &STRANGER);
    td_ble_core_connect(&CORE, 3, 0);
    /* The first two draws are at or above the largest multiple of 10^6 below 2^32 and are drawn again. */
    F.randoms[0] = 4294000000u;
    F.randoms[1] = 0xFFFFFFFFu;
    F.randoms[2] = 1234567u;
    uint32_t code = 0;
    CHECK(td_ble_core_passkey_action(&CORE, 3, TD_BLE_IO_DISPLAY, &code), "the displayed passkey was refused");
    CHECK(code == 234567 && F.random_next == 3, "passkey %u after %d draws", (unsigned)code, F.random_next);
    CHECK(F.passkey_on && F.passkey_code == 234567 && F.passkey_shows == 1, "passkey not on screen");
    td_ble_status_t status;
    td_ble_core_status(&CORE, &status);
    CHECK(status.passkey && status.stats.passkeys == 1, "status: passkey %d, %u shown", status.passkey,
          (unsigned)status.stats.passkeys);
    cJSON *reply = message("{\"m\":\"ble.state\"}");
    CHECK(cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "passkey")), "ble.state does not say passkey");
    char *text = cJSON_PrintUnformatted(reply);
    CHECK(text && !strstr(text, "234567"), "ble.state carries the code: %s", text ? text : "");
    free(text);
    cJSON_Delete(reply);
    CHECK(!td_ble_core_may_access(&CORE, 3), "the HID answered before pairing finished");

    /* The host typed it: pairing completes, the code goes, the link is trusted. */
    td_ble_core_pairing_complete(&CORE, 3, 0);
    CHECK(!F.passkey_on && F.passkey_hides == 1, "passkey still on screen after pairing completed");
    encrypt(3, true);
    CHECK(td_ble_core_may_access(&CORE, 3) && F.bond_count == 1, "the passkey bond is not trusted");
    td_ble_core_status(&CORE, &status);
    CHECK(!status.passkey && status.stats.pairings_refused == 0, "status after pairing");
}

/* The code leaves the screen whenever pairing stops, and only the displayed passkey inside the window pairs. */
static void test_passkey_cleared_and_refused(void)
{
    uint32_t code;
    /* It times out: the code goes and the link drops. */
    reset(1);
    boot_synced();
    td_ble_core_pair(&CORE, true);
    link_up(3, &STRANGER);
    td_ble_core_connect(&CORE, 3, 0);
    CHECK(td_ble_core_passkey_action(&CORE, 3, TD_BLE_IO_DISPLAY, &code), "passkey refused");
    advance(TD_BLE_PASSKEY_MS - 1);
    CHECK(F.passkey_on && F.terminates == 0, "passkey gone before its time");
    advance(1);
    CHECK(!F.passkey_on && F.terminates == 1 && F.last_terminated == 3, "passkey timeout: on %d, %d drops",
          F.passkey_on, F.terminates);
    CHECK(CORE.stats.pairings_refused == 1 && CORE.pairing, "timeout: %u refused, window %d",
          (unsigned)CORE.stats.pairings_refused, CORE.pairing);

    /* Pairing fails: the code goes and the link drops. */
    reset(1);
    boot_synced();
    td_ble_core_pair(&CORE, true);
    link_up(3, &STRANGER);
    td_ble_core_connect(&CORE, 3, 0);
    td_ble_core_passkey_action(&CORE, 3, TD_BLE_IO_DISPLAY, &code);
    td_ble_core_pairing_complete(&CORE, 3, 4);
    CHECK(!F.passkey_on && F.terminates == 1, "failed pairing: on %d, %d drops", F.passkey_on, F.terminates);
    CHECK(!F.timer_armed || F.timer_at == 1000 + TD_BLE_PAIR_MS, "the passkey's timer outlived it");

    /* The link drops: the code goes. A failed encryption takes it down too. */
    reset(1);
    boot_synced();
    td_ble_core_pair(&CORE, true);
    link_up(3, &STRANGER);
    td_ble_core_connect(&CORE, 3, 0);
    td_ble_core_passkey_action(&CORE, 3, TD_BLE_IO_DISPLAY, &code);
    link_down(3);
    CHECK(!F.passkey_on, "passkey on screen after its link dropped");
    link_up(4, &STRANGER);
    td_ble_core_connect(&CORE, 4, 0);
    td_ble_core_passkey_action(&CORE, 4, TD_BLE_IO_DISPLAY, &code);
    td_ble_core_enc_change(&CORE, 4, 13);
    CHECK(!F.passkey_on, "passkey on screen after encryption failed");

    /* The window closes, or Bluetooth turns off: the code goes. */
    link_down(4);
    link_up(5, &STRANGER);
    td_ble_core_connect(&CORE, 5, 0);
    td_ble_core_passkey_action(&CORE, 5, TD_BLE_IO_DISPLAY, &code);
    cJSON_Delete(message("{\"m\":\"ble.pair\",\"open\":false}"));
    CHECK(!F.passkey_on, "passkey on screen after the window closed");
    link_down(5);
    td_ble_core_pair(&CORE, true);
    link_up(6, &STRANGER);
    td_ble_core_connect(&CORE, 6, 0);
    td_ble_core_passkey_action(&CORE, 6, TD_BLE_IO_DISPLAY, &code);
    cJSON_Delete(message("{\"m\":\"ble.set\",\"enabled\":false}"));
    CHECK(!F.passkey_on, "passkey on screen after Bluetooth turned off");

    /* No window, another pairing method, or a second code at once: refused, nothing on screen. */
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    link_up(7, &HOST_A);
    td_ble_core_connect(&CORE, 7, 0);
    CHECK(!td_ble_core_passkey_action(&CORE, 7, TD_BLE_IO_DISPLAY, &code) && !F.passkey_on && F.terminates == 1,
          "a passkey outside the window");
    reset(1);
    boot_synced();
    td_ble_core_pair(&CORE, true);
    link_up(3, &STRANGER);
    td_ble_core_connect(&CORE, 3, 0);
    CHECK(!td_ble_core_passkey_action(&CORE, 3, TD_BLE_IO_OTHER, &code) && !F.passkey_on && F.terminates == 1,
          "numeric comparison or input accepted");
    link_down(3);
    link_up(4, &STRANGER);
    td_ble_core_connect(&CORE, 4, 0);
    CHECK(td_ble_core_passkey_action(&CORE, 4, TD_BLE_IO_DISPLAY, &code), "passkey refused");
    CHECK(!td_ble_core_passkey_action(&CORE, 9, TD_BLE_IO_DISPLAY, &code) && F.passkey_shows == 1,
          "a second passkey while one is on screen");
    CHECK(CORE.stats.pairings_refused == 2, "%u refused", (unsigned)CORE.stats.pairings_refused);
}

/* Keys without MITM protection or without Secure Connections never make a trusted link: the bond goes and the link
 * drops, whether the pairing just ended or an old bond reconnects. */
static void test_just_works_and_legacy_refused(void)
{
    const struct {
        bool authenticated;
        bool secure_connections;
        uint8_t key_size;
        const char *what;
    } cases[] = {
        { false, true, TD_BLE_KEY_SIZE, "Just Works" },
        { true, false, TD_BLE_KEY_SIZE, "legacy passkey" },
        { false, false, TD_BLE_KEY_SIZE, "legacy Just Works" },
        { true, true, 7, "a 7-byte key" },
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        reset(1);
        boot_synced();
        td_ble_core_pair(&CORE, true);
        link_up(3, &STRANGER);
        td_ble_core_connect(&CORE, 3, 0);
        encrypt_with(3, true, cases[i].authenticated, cases[i].secure_connections, cases[i].key_size);
        CHECK(!td_ble_core_may_access(&CORE, 3), "%s: the HID answers", cases[i].what);
        CHECK(F.bond_count == 0 && F.bond_deletes == 1, "%s: bond kept (%u bonds)", cases[i].what,
              (unsigned)F.bond_count);
        CHECK(F.terminates == 1 && F.last_terminated == 3, "%s: link kept", cases[i].what);
        CHECK(CORE.pairing && CORE.stats.pairings_refused == 1, "%s: window %d, %u refused", cases[i].what,
              CORE.pairing, (unsigned)CORE.stats.pairings_refused);
        td_ble_core_drain(&CORE);
        CHECK(F.notifies == 0, "%s: a report went out", cases[i].what);
    }

    /* An old Just Works bond reconnects, encrypted before CONNECT arrives. */
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    link_up(9, &HOST_A);
    fake_conn(9)->link = (td_ble_link_t){ .encrypted = true, .bonded = true, .authenticated = false,
                                          .secure_connections = true, .key_size = TD_BLE_KEY_SIZE, .peer = HOST_A };
    td_ble_core_connect(&CORE, 9, 0);
    CHECK(!td_ble_core_may_access(&CORE, 9) && F.bond_count == 0 && F.terminates == 1,
          "an unauthenticated bond reconnected: %u bonds, %d drops", (unsigned)F.bond_count, F.terminates);
}

/* The resolving list holds every bonded host and is written with the accept list, before advertising, so a host that
 * reconnects from a resolvable private address resolves to an identity the accept list lets in. */
static void test_resolving_list_follows_the_bonds(void)
{
    reset(1);
    add_bond(&HOST_A);
    add_bond(&HOST_B);
    boot_synced();
    CHECK(F.resolving_writes == 1 && F.resolving_count == 2 && addr_same(&F.resolving[0], &HOST_A) &&
              addr_same(&F.resolving[1], &HOST_B),
          "after sync: %d writes of %u hosts", F.resolving_writes, (unsigned)F.resolving_count);
    CHECK(F.adv_calls_at_resolving == 0 && F.advertising, "the resolving list came after advertising started");

    /* A new bond: written again, with all three, before reconnect advertising. */
    td_ble_core_pair(&CORE, true);
    link_up(3, &STRANGER);
    td_ble_core_connect(&CORE, 3, 0);
    pair_with_passkey(3);
    const int calls = F.adv_calls;
    link_down(3);
    CHECK(F.resolving_writes == 2 && F.resolving_count == 3 && addr_same(&F.resolving[2], &STRANGER),
          "after a new bond: %d writes of %u hosts", F.resolving_writes, (unsigned)F.resolving_count);
    CHECK(F.adv_calls_at_resolving == calls && F.advertising, "reconnect advertising before the resolving list");

    /* A forgotten bond leaves it. */
    size_t forgotten = 0;
    td_ble_core_forget(&CORE, &HOST_A, &forgotten);
    CHECK(F.resolving_writes == 3 && F.resolving_count == 2 && addr_same(&F.resolving[0], &HOST_B),
          "after forgetting: %d writes of %u hosts", F.resolving_writes, (unsigned)F.resolving_count);

    /* A controller that refuses it still advertises: hosts on identity addresses reconnect. */
    reset(1);
    add_bond(&HOST_A);
    F.resolving_rc = 7;
    boot_synced();
    CHECK(F.advertising && CORE.stats.resolve_failures == 1, "a resolving-list failure stopped advertising");
}

static void test_window_times_out_and_strangers_stay_out(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    td_ble_core_pair(&CORE, true);
    CHECK(F.adv_stops == 1 && F.advertising && F.adv_mode == TD_BLE_ADV_PAIRING,
          "opening the window did not swap reconnect for pairing advertising");
    adv_expires();
    CHECK(!CORE.pairing, "window open after 120 s");
    CHECK(F.advertising && F.adv_mode == TD_BLE_ADV_FAST, "no reconnect advertising after the window");

    /* Outside the window an unbonded host that slipped past the accept list is dropped without a Security Request. */
    link_up(4, &STRANGER);
    td_ble_core_connect(&CORE, 4, 0);
    CHECK(F.terminates == 1 && F.last_terminated == 4 && F.security_calls == 0, "a stranger outside the window");
    link_down(4);

    /* A bonded host whose link isn't encrypted yet gets one Security Request, and a failed encryption drops it. */
    link_up(5, &HOST_A);
    td_ble_core_connect(&CORE, 5, 0);
    CHECK(F.security_calls == 1, "%d Security Requests to a bonded host", F.security_calls);
    td_ble_core_enc_change(&CORE, 5, 0x13);
    CHECK(F.terminates == 2 && F.last_terminated == 5, "a failed encryption kept the link");
    CHECK(F.param_calls == 0, "connection parameters on a link that never encrypted");
    CHECK(!td_ble_core_may_access(&CORE, 5), "GATT access on an unencrypted link");
}

static void test_window_refused_when_full(void)
{
    reset(1);
    add_bond(&HOST_A);
    add_bond(&HOST_B);
    add_bond(&(td_ble_addr_t){ 0, { 1, 1, 1, 1, 1, 1 } });
    add_bond(&(td_ble_addr_t){ 0, { 2, 2, 2, 2, 2, 2 } });
    boot_synced();
    cJSON *reply = message("{\"m\":\"ble.pair\"}");
    CHECK(!ok(reply) && !strcmp(reason(reply), "full"), "pairing with %d bonds: %s", TD_BLE_BONDS_MAX, reason(reply));
    cJSON_Delete(reply);
    CHECK(F.adv_mode == TD_BLE_ADV_FAST, "a refused window changed advertising");
}

static void test_events_by_their_own_handle(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    link_up(9, &HOST_A);
    /* ENC_CHANGE for 9 before its CONNECT, and an event for a handle the host never had. */
    encrypt(9, false);
    td_ble_core_enc_change(&CORE, 0xFFFF, 0);
    CHECK(F.terminates == 1 && F.last_terminated == 0xFFFF, "an unknown handle was not dropped by its own handle");
    CHECK(td_ble_core_may_access(&CORE, 9), "the encrypted link lost its state to another handle's event");
    td_ble_core_connect(&CORE, 9, 0);
    CHECK(F.security_calls == 0 && F.param_calls == 1, "late CONNECT: security %d params %d", F.security_calls,
          F.param_calls);
    /* Protocol Mode is per connection and starts in Report Protocol. */
    CHECK(td_ble_core_protocol(&CORE, 9) == 1, "a connection did not start in Report Protocol");
    CHECK(td_ble_core_set_protocol(&CORE, 9, 0) && td_ble_core_protocol(&CORE, 9) == 0, "Protocol Mode write");
    CHECK(!td_ble_core_set_protocol(&CORE, 9, 2), "Protocol Mode 2 accepted");
    td_ble_core_subscribe(&CORE, 9, TD_BLE_IN_BOOT_KEYBOARD, true);
    td_ble_core_drain(&CORE);
    key_report_t a = key_report_kbd(0, 0x04);
    td_ble_core_report(&CORE, &a);
    td_ble_core_drain(&CORE);
    CHECK(F.last_input == TD_BLE_IN_BOOT_KEYBOARD && F.last_len == 8, "boot protocol report went to input %d",
          F.last_input);
    link_down(9);
    link_up(9, &HOST_A);
    td_ble_core_connect(&CORE, 9, 0);
    CHECK(td_ble_core_protocol(&CORE, 9) == 1, "Protocol Mode carried over to a new connection");
}

static void test_suspend_and_failed_notify(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    link_up(2, &HOST_A);
    encrypt(2, false);
    td_ble_core_connect(&CORE, 2, 0);
    td_ble_core_subscribe(&CORE, 2, TD_BLE_IN_KEYBOARD, true);
    td_ble_core_drain(&CORE);
    const int sent = F.notifies;
    CHECK(td_ble_core_control_point(&CORE, 2, 0), "Suspend refused");
    key_report_t a = key_report_kbd(0, 0x04);
    td_ble_core_report(&CORE, &a);
    td_ble_core_drain(&CORE);
    CHECK(F.notifies == sent, "a report went out while the host suspended");
    CHECK(td_ble_core_control_point(&CORE, 2, 1), "Exit Suspend refused");
    td_ble_core_drain(&CORE);
    CHECK(F.notifies > sent && F.last_data[2] == 0x04, "the state after Exit Suspend did not go out");

    /* A notification the host can't queue: the drain retries the current state 20 ms later, not the history. */
    F.notify_rc = 7;
    key_report_t release = key_report_kbd(0, 0);
    td_ble_core_report(&CORE, &a);
    td_ble_core_report(&CORE, &release);
    td_ble_core_drain(&CORE);
    CHECK(F.timer_armed && F.timer_at - F.now == 20, "no drain retry after a failed notify");
    F.notify_rc = 0;
    const int before = F.notifies;
    advance(20);
    CHECK(F.notifies > before && F.last_data[2] == 0, "the retry did not send the released state");
}

static void test_forget(void)
{
    reset(1);
    add_bond(&HOST_A);
    add_bond(&HOST_B);
    boot_synced();
    CHECK(F.accept_writes == 1 && F.accept_count == 2, "accept list with two bonds");
    cJSON *reply = message("{\"m\":\"ble.forget\",\"address\":\"c6:15:14:13:12:11\"}");
    CHECK(ok(reply), "forget by address failed: %s", reason(reply));
    CHECK(cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(reply, "forgotten")) == 1, "forgotten count");
    cJSON *bonds = cJSON_GetObjectItemCaseSensitive(reply, "bonds");
    CHECK(cJSON_GetArraySize(bonds) == 1, "bonds after forget: %d", cJSON_GetArraySize(bonds));
    const char *left = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(cJSON_GetArrayItem(bonds, 0), "address"));
    CHECK(left && !strcmp(left, "06:05:04:03:02:01"), "the wrong bond stayed: %s", left ? left : "none");
    cJSON_Delete(reply);
    CHECK(F.adv_stops == 1 && F.advertising && F.accept_writes == 2 && F.accept_count == 1,
          "after forget: stops %d advertising %d writes %d", F.adv_stops, F.advertising, F.accept_writes);

    reply = message("{\"m\":\"ble.forget\",\"address\":\"aa:bb:cc:dd:ee:ff\"}");
    CHECK(ok(reply) && cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(reply, "forgotten")) == 0,
          "forgetting an unknown host");
    cJSON_Delete(reply);
    CHECK(F.accept_writes == 2, "an unchanged bond set rewrote the accept list");

    reply = message("{\"m\":\"ble.forget\",\"address\":null}");
    CHECK(ok(reply) && cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(reply, "forgotten")) == 1, "forget all");
    cJSON_Delete(reply);
    CHECK(F.bond_count == 0 && !F.advertising, "advertising with no bonds left");
}

static void test_repeat_pairing_only_in_window(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    link_up(6, &HOST_A);
    td_ble_core_connect(&CORE, 6, 0);
    CHECK(!td_ble_core_repeat_pairing(&CORE, 6), "a bonded host re-paired outside the window");
    CHECK(F.keys_deletes == 0 && F.bond_count == 1, "a bond deleted outside the window");
    td_ble_core_pair(&CORE, true);
    CHECK(td_ble_core_repeat_pairing(&CORE, 6), "re-pairing refused inside the window");
    CHECK(F.keys_deletes == 1 && F.terminates == 0, "re-pairing dropped the link or kept the old keys");
}

static void test_messages_reject_bad_requests(void)
{
    reset(1);
    boot_synced();
    const char *bad[] = {
        "{\"m\":\"ble.set\"}",
        "{\"m\":\"ble.set\",\"enabled\":1}",
        "{\"m\":\"ble.set\",\"enabled\":true,\"x\":1}",
        "{\"m\":\"ble.pair\",\"open\":\"yes\"}",
        "{\"m\":\"ble.forget\",\"address\":\"01:02:03:04:05\"}",
        "{\"m\":\"ble.forget\",\"address\":\"01-02-03-04-05-06\"}",
        "{\"m\":\"ble.state\",\"x\":true}",
    };
    for (size_t i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        cJSON *reply = message(bad[i]);
        CHECK(!ok(reply) && !strcmp(reason(reply), "args"), "%s: %s", bad[i], reason(reply));
        cJSON_Delete(reply);
    }
    cJSON *reply = message("{\"m\":\"ble.scan\"}");
    CHECK(!ok(reply) && !strcmp(reason(reply), "unsupported"), "ble.scan: %s", reason(reply));
    cJSON_Delete(reply);
    CHECK(F.stored == 1 && F.adv_calls == 0, "a bad request changed something");

    /* Starting: commands that need the host wait for sync. */
    reset(1);
    td_ble_core_boot(&CORE);
    reply = message("{\"m\":\"ble.pair\"}");
    CHECK(!ok(reply) && !strcmp(reason(reply), "starting"), "ble.pair before sync: %s", reason(reply));
    cJSON_Delete(reply);
    /* A failed start reports itself. */
    reset(1);
    F.stack_start_rc = 5;
    td_ble_core_boot(&CORE);
    reply = message("{\"m\":\"ble.state\"}");
    CHECK(!strcmp(cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, "state")), "failed"), "failed state");
    cJSON_Delete(reply);
}

static const char *state_of(const cJSON *reply)
{
    const char *s = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, "state"));
    return s ? s : "";
}

static void test_failed_start_then_off(void)
{
    reset(0);
    td_ble_core_boot(&CORE);
    F.stack_start_rc = 5;
    cJSON *reply = message("{\"m\":\"ble.set\",\"enabled\":true}");
    CHECK(!ok(reply) && !strcmp(reason(reply), "radio") && !strcmp(state_of(reply), "failed"), "failed start: %s %s",
          reason(reply), state_of(reply));
    cJSON_Delete(reply);
    /* Turning it off after a failed start reads off, not failed. */
    reply = message("{\"m\":\"ble.set\",\"enabled\":false}");
    CHECK(ok(reply) && !strcmp(state_of(reply), "off"), "off after a failed start: %s", state_of(reply));
    CHECK(!cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "enabled")), "still enabled after off");
    cJSON_Delete(reply);
    reply = message("{\"m\":\"ble.state\"}");
    CHECK(!strcmp(state_of(reply), "off"), "ble.state after off: %s", state_of(reply));
    cJSON_Delete(reply);
    /* A later start that works reads starting until the host syncs. */
    F.stack_start_rc = 0;
    reply = message("{\"m\":\"ble.set\",\"enabled\":true}");
    CHECK(ok(reply) && !strcmp(state_of(reply), "starting"), "start after a failure: %s", state_of(reply));
    cJSON_Delete(reply);
}

static void test_host_reset_keeps_the_bonds(void)
{
    reset(1);
    add_bond(&HOST_A);
    boot_synced();
    td_ble_core_host_reset(&CORE);
    /* The host is starting again, and the stored bonds are still listed. */
    cJSON *reply = message("{\"m\":\"ble.state\"}");
    const cJSON *bonds = cJSON_GetObjectItemCaseSensitive(reply, "bonds");
    CHECK(!strcmp(state_of(reply), "starting") && cJSON_GetArraySize(bonds) == 1,
          "after a host reset: %s with %d bonds", state_of(reply), cJSON_GetArraySize(bonds));
    cJSON_Delete(reply);
}

static void test_caps(void)
{
    cJSON *caps = cJSON_CreateObject();
    td_ble_caps(caps);
    CHECK(cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "ble_hid")) == TD_BLE_BONDS_MAX, "ble_hid cap");
    cJSON_Delete(caps);
    td_ble_addr_t addr;
    char text[18];
    CHECK(td_ble_addr_parse("C6:15:14:13:12:11", &addr), "upper-case address refused");
    td_ble_addr_format(&addr, text);
    CHECK(!strcmp(text, "c6:15:14:13:12:11") && addr.val[0] == 0x11, "address round trip: %s", text);
}

int main(void)
{
    test_off_unless_stored_on();
    test_no_bonds_no_advertising();
    test_reboot_reconnect_replay();
    test_connect_races_the_end_of_advertising();
    test_fast_then_slow();
    test_failed_start_backs_off();
    test_pairing_window();
    test_window_times_out_and_strangers_stay_out();
    test_passkey_flow();
    test_passkey_cleared_and_refused();
    test_just_works_and_legacy_refused();
    test_resolving_list_follows_the_bonds();
    test_window_refused_when_full();
    test_events_by_their_own_handle();
    test_suspend_and_failed_notify();
    test_forget();
    test_repeat_pairing_only_in_window();
    test_messages_reject_bad_requests();
    test_failed_start_then_off();
    test_host_reset_keeps_the_bonds();
    test_caps();
    if (g_failures) {
        printf("ble core: %d failures\n", g_failures);
        return 1;
    }
    printf("ble core: 21 tests passed\n");
    return 0;
}
