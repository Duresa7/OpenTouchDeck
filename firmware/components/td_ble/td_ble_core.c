#include "td_ble_core.h"
#include <string.h>

#define PORT(core) ((core)->port)
#define CTX(core) ((core)->port->ctx)
/* Advertising that would run for less than this ends its phase instead, so a duration that runs out a little early
 * never restarts the same phase for a few milliseconds. */
#define PHASE_SLACK_MS 100u
/* A notification the host stack couldn't queue is retried this soon, from the queue's current state. */
#define DRAIN_RETRY_MS 20u

static bool reached(uint32_t now, uint32_t deadline)
{
    return (int32_t)(now - deadline) >= 0;
}

static uint32_t left(uint32_t now, uint32_t deadline)
{
    return reached(now, deadline) ? 0 : deadline - now;
}

static bool addr_eq(const td_ble_addr_t *a, const td_ble_addr_t *b)
{
    return a->type == b->type && memcmp(a->val, b->val, sizeof a->val) == 0;
}

/* ---- The report queue. Call under the port's lock. ---- */

static void queue_resync(td_ble_queue_t *q)
{
    q->head = 0;
    q->count = 0;
    q->sync = q->online ? 3 : 0;
}

static void queue_online(td_ble_queue_t *q, bool online)
{
    if (q->online != online) {
        q->online = online;
        queue_resync(q);
    }
}

static void queue_push(td_ble_queue_t *q, const key_report_t *report)
{
    q->current[report->type == KEY_REPORT_CON ? 1 : 0] = *report;
    if (!q->online) {
        return;
    }
    if (q->count == TD_BLE_QUEUE_SIZE) {
        /* Full: drop the history and send the state as it is now, so a release is never lost. */
        queue_resync(q);
        return;
    }
    q->entries[(q->head + q->count++) % TD_BLE_QUEUE_SIZE] = *report;
}

static bool queue_next(td_ble_queue_t *q, key_report_t *report)
{
    if (!q->online) {
        return false;
    }
    for (unsigned i = 0; i < 2; i++) {
        if (q->sync & (1u << i)) {
            q->sync &= ~(1u << i);
            *report = q->current[i];
            return true;
        }
    }
    if (q->count == 0) {
        return false;
    }
    *report = q->entries[q->head];
    q->head = (q->head + 1) % TD_BLE_QUEUE_SIZE;
    q->count--;
    return true;
}

static void resync(td_ble_core_t *core)
{
    PORT(core)->lock(CTX(core));
    queue_resync(&core->queue);
    PORT(core)->unlock(CTX(core));
}

/* ---- Connections, each by its own handle. ---- */

static td_ble_conn_t *slot_find(td_ble_core_t *core, uint16_t conn)
{
    for (size_t i = 0; i < TD_BLE_CONNS_MAX; i++) {
        if (core->conns[i].used && core->conns[i].handle == conn) {
            return &core->conns[i];
        }
    }
    return NULL;
}

/* The slot for a handle, made by the first event that names it, whichever event that is. */
static td_ble_conn_t *slot_get(td_ble_core_t *core, uint16_t conn)
{
    td_ble_conn_t *slot = slot_find(core, conn);
    for (size_t i = 0; !slot && i < TD_BLE_CONNS_MAX; i++) {
        if (!core->conns[i].used) {
            slot = &core->conns[i];
            /* Every connection starts in Report Protocol and not suspended (HIDS 2.4.1.1). */
            *slot = (td_ble_conn_t){ .used = true, .handle = conn, .protocol = 1 };
        }
    }
    return slot;
}

static bool slot_trusted(const td_ble_conn_t *slot)
{
    return slot && slot->encrypted && slot->bonded;
}

/* The connection that gets the reports: up, encrypted, bonded, not suspended, and listening on some input. */
static td_ble_conn_t *ready_slot(td_ble_core_t *core)
{
    for (size_t i = 0; i < TD_BLE_CONNS_MAX; i++) {
        td_ble_conn_t *slot = &core->conns[i];
        if (slot->used && slot->connected && slot_trusted(slot) && !slot->suspended &&
            (slot->notify[TD_BLE_IN_KEYBOARD] || slot->notify[TD_BLE_IN_CONSUMER] ||
             slot->notify[TD_BLE_IN_BOOT_KEYBOARD])) {
            return slot;
        }
    }
    return NULL;
}

static void update_online(td_ble_core_t *core)
{
    const bool online = core->enabled && ready_slot(core) != NULL;
    PORT(core)->lock(CTX(core));
    queue_online(&core->queue, online);
    PORT(core)->unlock(CTX(core));
}

/* ---- Bonds. ---- */

static bool is_bonded(const td_ble_core_t *core, const td_ble_addr_t *peer)
{
    for (size_t i = 0; i < core->bond_count; i++) {
        if (addr_eq(&core->bonds[i], peer)) {
            return true;
        }
    }
    return false;
}

/* Reads the bonds from the store. True when they changed, which makes the accept list due. */
static bool refresh_bonds(td_ble_core_t *core)
{
    td_ble_addr_t bonds[TD_BLE_BONDS_MAX];
    size_t count = PORT(core)->bonds(CTX(core), bonds);
    if (count > TD_BLE_BONDS_MAX) {
        count = TD_BLE_BONDS_MAX;
    }
    const bool changed = count != core->bond_count || memcmp(bonds, core->bonds, count * sizeof bonds[0]) != 0;
    if (changed) {
        memcpy(core->bonds, bonds, count * sizeof bonds[0]);
        core->bond_count = count;
        core->accept_dirty = true;
    }
    return changed;
}

/* ---- One one-shot timer serves the pairing window's end, the backoff retry and a drain retry. ---- */

static void arm_timer(td_ble_core_t *core)
{
    if (!core->running) {
        PORT(core)->timer_cancel(CTX(core));
        return;
    }
    const struct {
        bool on;
        uint32_t at;
    } due[] = { { core->pairing, core->pair_until }, { core->retry_armed, core->retry_at },
                { core->drain_due, core->drain_at } };
    bool any = false;
    uint32_t deadline = 0;
    for (size_t i = 0; i < sizeof due / sizeof due[0]; i++) {
        if (due[i].on && (!any || (int32_t)(due[i].at - deadline) < 0)) {
            deadline = due[i].at;
            any = true;
        }
    }
    if (any) {
        PORT(core)->timer(CTX(core), left(PORT(core)->now_ms(CTX(core)), deadline));
    } else {
        PORT(core)->timer_cancel(CTX(core));
    }
}

/* ---- Advertising: one state machine, started only from events. ---- */

static void backoff(td_ble_core_t *core, uint32_t now)
{
    core->stats.adv_failures++;
    core->retry_ms = core->retry_ms ? core->retry_ms * 2 : TD_BLE_RETRY_MIN_MS;
    if (core->retry_ms > TD_BLE_RETRY_MAX_MS) {
        core->retry_ms = TD_BLE_RETRY_MAX_MS;
    }
    core->retry_armed = true;
    core->retry_at = now + core->retry_ms;
}

/* Ends the pairing window: stops its advertising and drops a host that connected but never bonded. */
static void close_window(td_ble_core_t *core)
{
    if (!core->pairing) {
        return;
    }
    core->pairing = false;
    if (core->advertising && core->adv_mode == TD_BLE_ADV_PAIRING) {
        PORT(core)->adv_stop(CTX(core));
        core->advertising = false;
    }
    for (size_t i = 0; i < TD_BLE_CONNS_MAX; i++) {
        const td_ble_conn_t *slot = &core->conns[i];
        if (slot->used && !slot_trusted(slot)) {
            PORT(core)->terminate(CTX(core), slot->handle);
        }
    }
    core->episode = false;
}

static void kick(td_ble_core_t *core)
{
    if (!core->enabled || !core->running || !core->synced || core->advertising || core->retry_armed) {
        return;
    }
    /* A connection object exists from Connection Complete on, well before NimBLE delivers CONNECT. While there is one,
     * the only connection slot is taken and advertising must not start. */
    if (PORT(core)->conn_count(CTX(core)) > 0) {
        return;
    }
    const uint32_t now = PORT(core)->now_ms(CTX(core));
    if (core->pairing && left(now, core->pair_until) < PHASE_SLACK_MS) {
        close_window(core);
    }
    td_ble_adv_t mode;
    uint32_t duration;
    if (core->pairing) {
        mode = TD_BLE_ADV_PAIRING;
        duration = left(now, core->pair_until);
    } else {
        if (core->bond_count == 0) {
            return;
        }
        if (core->accept_dirty) {
            core->stats.accept_writes++;
            if (PORT(core)->accept_list(CTX(core), core->bonds, core->bond_count) != 0) {
                backoff(core, now);
                return;
            }
            core->accept_dirty = false;
        }
        if (!core->episode) {
            core->episode = true;
            core->fast_until = now + TD_BLE_FAST_MS;
        }
        if (left(now, core->fast_until) >= PHASE_SLACK_MS) {
            mode = TD_BLE_ADV_FAST;
            duration = left(now, core->fast_until);
        } else {
            mode = TD_BLE_ADV_SLOW;
            duration = 0;
        }
    }
    if (PORT(core)->adv_start(CTX(core), mode, duration) != 0) {
        backoff(core, now);
        return;
    }
    core->stats.adv_starts++;
    core->advertising = true;
    core->adv_mode = mode;
    core->retry_ms = 0;
}

/* Every event ends here: advertising if it is due, then the timer for whatever comes next. */
static void settle(td_ble_core_t *core)
{
    kick(core);
    arm_timer(core);
}

/* At most one request per connection, once CONNECT has arrived and the link is encrypted with a bonded host. A
 * refusal stands: the host's values stay. */
static void request_params(td_ble_core_t *core, td_ble_conn_t *slot)
{
    if (slot->connected && slot_trusted(slot) && !slot->params_sent) {
        slot->params_sent = true;
        core->stats.param_requests++;
        PORT(core)->conn_params(CTX(core), slot->handle);
    }
}

/* ---- Life cycle. ---- */

void td_ble_core_init(td_ble_core_t *core, const td_ble_port_t *port)
{
    *core = (td_ble_core_t){ .port = port };
    core->queue.current[0] = key_report_kbd(0, 0);
    core->queue.current[1] = key_report_con(0);
}

static td_ble_result_t start(td_ble_core_t *core)
{
    if (core->running) {
        return TD_BLE_OK;
    }
    core->stats.stack_starts++;
    if (PORT(core)->stack_start(CTX(core)) != 0) {
        core->start_failed = true;
        return TD_BLE_ERR_RADIO;
    }
    core->running = true;
    core->start_failed = false;
    core->synced = false;
    return TD_BLE_OK;
}

static void stop(td_ble_core_t *core)
{
    if (!core->running) {
        return;
    }
    core->pairing = false;
    core->retry_armed = false;
    core->drain_due = false;
    PORT(core)->timer_cancel(CTX(core));
    PORT(core)->stack_stop(CTX(core));
    core->running = false;
    core->synced = false;
    core->advertising = false;
    core->episode = false;
    core->retry_ms = 0;
    core->bond_count = 0;
    memset(core->conns, 0, sizeof core->conns);
    update_online(core);
}

void td_ble_core_boot(td_ble_core_t *core)
{
    core->enabled = PORT(core)->load_enabled(CTX(core));
    if (core->enabled) {
        start(core);
    }
}

/* ---- Host events. ---- */

void td_ble_core_synced(td_ble_core_t *core)
{
    if (!core->running) {
        return;
    }
    core->synced = true;
    core->advertising = false;
    core->episode = false;
    refresh_bonds(core);
    /* The controller's accept list is empty after a start, so it is due once whatever the bonds are. */
    core->accept_dirty = true;
    settle(core);
}

void td_ble_core_host_reset(td_ble_core_t *core)
{
    core->synced = false;
    core->advertising = false;
    memset(core->conns, 0, sizeof core->conns);
    update_online(core);
}

void td_ble_core_connect(td_ble_core_t *core, uint16_t conn, int status)
{
    /* A connection, or a failed attempt at one, ends legacy advertising. */
    core->advertising = false;
    if (status != 0) {
        settle(core);
        return;
    }
    td_ble_conn_t *slot = slot_get(core, conn);
    td_ble_link_t link;
    if (!slot || !PORT(core)->link(CTX(core), conn, &link)) {
        PORT(core)->terminate(CTX(core), conn);
        return;
    }
    slot->connected = true;
    slot->peer = link.peer;
    if (!core->enabled) {
        PORT(core)->terminate(CTX(core), conn);
        return;
    }
    if (link.encrypted) {
        /* ENC_CHANGE came first, as it does when the host encrypts with a stored key before NimBLE delivers CONNECT.
         * No Security Request. */
        slot->encrypted = true;
        slot->bonded = link.bonded;
        if (!link.bonded) {
            PORT(core)->terminate(CTX(core), conn);
            return;
        }
        request_params(core, slot);
    } else if (!core->pairing && !is_bonded(core, &link.peer)) {
        /* A stranger outside the pairing window. The accept list keeps them out; this is the second check. */
        PORT(core)->terminate(CTX(core), conn);
        return;
    } else if (!slot->security_sent) {
        slot->security_sent = true;
        core->stats.security_requests++;
        if (PORT(core)->security(CTX(core), conn) != 0) {
            PORT(core)->terminate(CTX(core), conn);
            return;
        }
    }
    update_online(core);
    arm_timer(core);
}

void td_ble_core_enc_change(td_ble_core_t *core, uint16_t conn, int status)
{
    td_ble_conn_t *slot = slot_get(core, conn);
    td_ble_link_t link;
    const bool ok = slot && status == 0 && PORT(core)->link(CTX(core), conn, &link) && link.encrypted && link.bonded;
    if (!ok) {
        if (slot) {
            slot->encrypted = false;
        }
        PORT(core)->terminate(CTX(core), conn);
        update_online(core);
        return;
    }
    slot->encrypted = true;
    slot->bonded = true;
    slot->peer = link.peer;
    if (refresh_bonds(core) && core->pairing) {
        /* A new bond: the window did its job. Its advertising already ended with this connection. */
        core->pairing = false;
        core->advertising = false;
        core->episode = false;
    }
    request_params(core, slot);
    update_online(core);
    arm_timer(core);
}

void td_ble_core_disconnect(td_ble_core_t *core, uint16_t conn)
{
    td_ble_conn_t *slot = slot_find(core, conn);
    if (slot) {
        memset(slot, 0, sizeof *slot);
    }
    /* With one connection slot nothing advertises while connected, so whatever ran ended with the connection, even
     * when its CONNECT never arrived. A new reconnect episode starts fast. */
    core->advertising = false;
    core->episode = false;
    update_online(core);
    if (core->pairing && left(PORT(core)->now_ms(CTX(core)), core->pair_until) < PHASE_SLACK_MS) {
        close_window(core);
    }
    settle(core);
}

void td_ble_core_adv_complete(td_ble_core_t *core, bool expired)
{
    const uint32_t now = PORT(core)->now_ms(CTX(core));
    const td_ble_adv_t mode = core->adv_mode;
    core->advertising = false;
    if (!expired) {
        backoff(core, now);
    } else if (mode == TD_BLE_ADV_PAIRING && left(now, core->pair_until) < PHASE_SLACK_MS) {
        close_window(core);
    }
    settle(core);
}

void td_ble_core_subscribe(td_ble_core_t *core, uint16_t conn, td_ble_input_t input, bool notify)
{
    td_ble_conn_t *slot = slot_get(core, conn);
    if (!slot || input >= TD_BLE_IN_COUNT) {
        return;
    }
    slot->notify[input] = notify;
    if (notify) {
        resync(core);
    }
    update_online(core);
}

void td_ble_core_timer(td_ble_core_t *core)
{
    const uint32_t now = PORT(core)->now_ms(CTX(core));
    if (core->pairing && reached(now, core->pair_until)) {
        close_window(core);
    }
    if (core->retry_armed && reached(now, core->retry_at)) {
        core->retry_armed = false;
    }
    if (core->drain_due && reached(now, core->drain_at)) {
        core->drain_due = false;
        td_ble_core_drain(core);
    }
    settle(core);
}

bool td_ble_core_repeat_pairing(td_ble_core_t *core, uint16_t conn)
{
    const td_ble_conn_t *slot = slot_find(core, conn);
    if (!core->pairing || !slot) {
        return false;
    }
    return PORT(core)->keys_delete(CTX(core), &slot->peer) == 0;
}

/* ---- GATT access. ---- */

bool td_ble_core_may_access(const td_ble_core_t *core, uint16_t conn)
{
    for (size_t i = 0; i < TD_BLE_CONNS_MAX; i++) {
        if (core->conns[i].used && core->conns[i].handle == conn) {
            return slot_trusted(&core->conns[i]);
        }
    }
    return false;
}

uint8_t td_ble_core_protocol(const td_ble_core_t *core, uint16_t conn)
{
    for (size_t i = 0; i < TD_BLE_CONNS_MAX; i++) {
        if (core->conns[i].used && core->conns[i].handle == conn) {
            return core->conns[i].protocol;
        }
    }
    return 1;
}

uint8_t td_ble_core_leds(const td_ble_core_t *core, uint16_t conn)
{
    for (size_t i = 0; i < TD_BLE_CONNS_MAX; i++) {
        if (core->conns[i].used && core->conns[i].handle == conn) {
            return core->conns[i].leds;
        }
    }
    return 0;
}

bool td_ble_core_set_protocol(td_ble_core_t *core, uint16_t conn, uint8_t mode)
{
    td_ble_conn_t *slot = slot_find(core, conn);
    if (!slot || mode > 1) {
        return false;
    }
    slot->protocol = mode;
    resync(core);
    return true;
}

bool td_ble_core_control_point(td_ble_core_t *core, uint16_t conn, uint8_t command)
{
    td_ble_conn_t *slot = slot_find(core, conn);
    if (!slot || command > 1) {
        return false;
    }
    slot->suspended = command == 0;
    update_online(core);
    return true;
}

void td_ble_core_set_leds(td_ble_core_t *core, uint16_t conn, uint8_t leds)
{
    td_ble_conn_t *slot = slot_find(core, conn);
    if (slot) {
        slot->leds = leds;
    }
}

size_t td_ble_core_current(td_ble_core_t *core, td_ble_input_t input, uint8_t out[TD_BLE_KEYBOARD_BYTES])
{
    PORT(core)->lock(CTX(core));
    const key_report_t report = core->queue.current[input == TD_BLE_IN_CONSUMER ? 1 : 0];
    PORT(core)->unlock(CTX(core));
    td_ble_input_t ignored;
    size_t len = td_ble_encode(&report, input == TD_BLE_IN_BOOT_KEYBOARD ? 0 : 1, out, &ignored);
    if (len == 0) {
        memset(out, 0, TD_BLE_CONSUMER_BYTES);
        len = TD_BLE_CONSUMER_BYTES;
    }
    return len;
}

/* ---- Commands. ---- */

td_ble_result_t td_ble_core_set_enabled(td_ble_core_t *core, bool on)
{
    if (on == core->enabled && on == core->running) {
        return TD_BLE_OK;
    }
    if (on != core->enabled) {
        if (PORT(core)->save_enabled(CTX(core), on) != 0) {
            return TD_BLE_ERR_STORAGE;
        }
        core->enabled = on;
    }
    if (on) {
        return start(core);
    }
    stop(core);
    return TD_BLE_OK;
}

td_ble_result_t td_ble_core_pair(td_ble_core_t *core, bool open)
{
    if (!core->enabled || !core->running) {
        return TD_BLE_ERR_OFF;
    }
    if (!core->synced) {
        return TD_BLE_ERR_STARTING;
    }
    if (!open) {
        close_window(core);
        settle(core);
        return TD_BLE_OK;
    }
    refresh_bonds(core);
    if (!core->pairing && core->bond_count >= TD_BLE_BONDS_MAX) {
        return TD_BLE_ERR_FULL;
    }
    core->pairing = true;
    core->pair_until = PORT(core)->now_ms(CTX(core)) + TD_BLE_PAIR_MS;
    if (core->advertising && core->adv_mode != TD_BLE_ADV_PAIRING) {
        PORT(core)->adv_stop(CTX(core));
        core->advertising = false;
    }
    /* A command is an event of its own, so it may start advertising now even after a failed start. */
    core->retry_armed = false;
    core->retry_ms = 0;
    settle(core);
    return TD_BLE_OK;
}

td_ble_result_t td_ble_core_forget(td_ble_core_t *core, const td_ble_addr_t *peer, size_t *forgotten)
{
    *forgotten = 0;
    if (!core->enabled || !core->running) {
        return TD_BLE_ERR_OFF;
    }
    if (!core->synced) {
        return TD_BLE_ERR_STARTING;
    }
    refresh_bonds(core);
    td_ble_addr_t bonds[TD_BLE_BONDS_MAX];
    const size_t count = core->bond_count;
    memcpy(bonds, core->bonds, sizeof bonds);
    td_ble_result_t result = TD_BLE_OK;
    for (size_t i = 0; i < count; i++) {
        if (peer && !addr_eq(peer, &bonds[i])) {
            continue;
        }
        /* Deleting a bond also drops a link to that host. */
        if (PORT(core)->bond_delete(CTX(core), &bonds[i]) == 0) {
            (*forgotten)++;
        } else {
            result = TD_BLE_ERR_STORAGE;
        }
    }
    if (refresh_bonds(core) && core->advertising && core->adv_mode != TD_BLE_ADV_PAIRING) {
        /* The accept list can only change while advertising is off. */
        PORT(core)->adv_stop(CTX(core));
        core->advertising = false;
        core->episode = false;
    }
    settle(core);
    return result;
}

void td_ble_core_status(const td_ble_core_t *core, td_ble_status_t *out)
{
    *out = (td_ble_status_t){
        .enabled = core->enabled,
        .running = core->running,
        .start_failed = core->start_failed,
        .synced = core->synced,
        .pairing = core->pairing,
        .advertising = core->advertising,
        .adv_mode = core->adv_mode,
        .bond_count = core->bond_count,
        .stats = core->stats,
    };
    if (core->pairing && core->running) {
        out->pairing_ms = left(PORT(core)->now_ms(CTX(core)), core->pair_until);
    }
    memcpy(out->bonds, core->bonds, sizeof out->bonds);
    for (size_t b = 0; b < core->bond_count; b++) {
        for (size_t i = 0; i < TD_BLE_CONNS_MAX; i++) {
            const td_ble_conn_t *slot = &core->conns[i];
            if (slot->used && slot_trusted(slot) && addr_eq(&slot->peer, &core->bonds[b])) {
                out->bond_connected[b] = true;
            }
        }
    }
}

/* ---- Reports. ---- */

bool td_ble_core_report(td_ble_core_t *core, const key_report_t *report)
{
    PORT(core)->lock(CTX(core));
    queue_push(&core->queue, report);
    const bool pending = core->queue.online && (core->queue.sync || core->queue.count);
    PORT(core)->unlock(CTX(core));
    return pending;
}

void td_ble_core_drain(td_ble_core_t *core)
{
    const td_ble_conn_t *slot = ready_slot(core);
    if (!slot) {
        return;
    }
    for (;;) {
        key_report_t report;
        PORT(core)->lock(CTX(core));
        const bool have = queue_next(&core->queue, &report);
        PORT(core)->unlock(CTX(core));
        if (!have) {
            return;
        }
        uint8_t bytes[TD_BLE_KEYBOARD_BYTES];
        td_ble_input_t input;
        const size_t len = td_ble_encode(&report, slot->protocol, bytes, &input);
        if (len == 0 || !slot->notify[input]) {
            continue;
        }
        if (PORT(core)->notify(CTX(core), slot->handle, input, bytes, len) != 0) {
            resync(core);
            core->drain_due = true;
            core->drain_at = PORT(core)->now_ms(CTX(core)) + DRAIN_RETRY_MS;
            arm_timer(core);
            return;
        }
    }
}
