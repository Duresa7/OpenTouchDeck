#include "td_netlink.h"
#include <string.h>

/* The largest multiple of 10^6 below 2^32, so code draws stay uniform. */
#define CODE_DRAW_LIMIT 4294000000u
#define REQUEST_MIN (TD_NETLINK_COMPANION_ID_LEN + 2)
#define REQUEST_MAX (TD_NETLINK_COMPANION_ID_LEN + 1 + TD_NETLINK_NAME_MAX)
#define OPEN_FIELDS (TD_NETLINK_OPEN_TAG_LEN + TD_NETLINK_SHARE_LEN)
#define PROOF_FIELDS TD_NETLINK_H_LEN
#define START_FIELDS (TD_NETLINK_SRP_LEN + 32)

const uint8_t td_netlink_busy_item[5] = { 0x00, 0x03, TD_NETLINK_ERROR, TD_NETLINK_VERSION, TD_NETLINK_BUSY };

static uint32_t now_ms(const td_netlink_deck_t *deck)
{
    return deck->env.clock(deck->env.ctx);
}

static uint32_t since(const td_netlink_deck_t *deck, uint32_t start)
{
    return now_ms(deck) - start;
}

/* True while `until` is still ahead of now, on the wrapping clock. */
static bool before(const td_netlink_deck_t *deck, uint32_t until)
{
    return (int32_t)(until - now_ms(deck)) > 0;
}

static void put_item(td_netlink_out_t *out, uint8_t kind, const td_netlink_part_t *parts, size_t count)
{
    size_t at = 4;
    for (size_t i = 0; i < count; i++) {
        memcpy(out->item + at, parts[i].data, parts[i].len);
        at += parts[i].len;
    }
    out->item[0] = (uint8_t)((at - 2) >> 8);
    out->item[1] = (uint8_t)(at - 2);
    out->item[2] = kind;
    out->item[3] = TD_NETLINK_VERSION;
    out->len = (uint16_t)at;
}

/* ---- Names ---- */

/* Unicode 17.0's General_Category Cf, pinned by net-v2.md. */
static const uint32_t FORMAT_RANGES[][2] = {
    { 0x00ad, 0x00ad }, { 0x0600, 0x0605 }, { 0x061c, 0x061c }, { 0x06dd, 0x06dd }, { 0x070f, 0x070f },
    { 0x0890, 0x0891 }, { 0x08e2, 0x08e2 }, { 0x180e, 0x180e }, { 0x200b, 0x200f }, { 0x202a, 0x202e },
    { 0x2060, 0x2064 }, { 0x2066, 0x206f }, { 0xfeff, 0xfeff }, { 0xfff9, 0xfffb }, { 0x110bd, 0x110bd },
    { 0x110cd, 0x110cd }, { 0x13430, 0x1343f }, { 0x1bca0, 0x1bca3 }, { 0x1d173, 0x1d17a }, { 0xe0001, 0xe0001 },
    { 0xe0020, 0xe007f },
};
static const uint32_t HANGUL_FILLERS[] = { 0x115f, 0x1160, 0x3164, 0xffa0 };

static bool refused_code_point(uint32_t c)
{
    if (c < 0x20 || (c >= 0x7f && c <= 0x9f)) {
        return true;
    }
    for (size_t i = 0; i < sizeof FORMAT_RANGES / sizeof FORMAT_RANGES[0]; i++) {
        if (c >= FORMAT_RANGES[i][0] && c <= FORMAT_RANGES[i][1]) {
            return true;
        }
    }
    for (size_t i = 0; i < sizeof HANGUL_FILLERS / sizeof HANGUL_FILLERS[0]; i++) {
        if (c == HANGUL_FILLERS[i]) {
            return true;
        }
    }
    return false;
}

bool td_netlink_name_valid(const uint8_t *s, size_t n)
{
    if (n < 1 || n > TD_NETLINK_NAME_MAX) {
        return false;
    }
    size_t i = 0;
    while (i < n) {
        uint32_t c = s[i];
        size_t extra;
        uint32_t min;
        if (c < 0x80) {
            extra = 0;
            min = 0;
        } else if ((c & 0xe0) == 0xc0) {
            extra = 1;
            min = 0x80;
            c &= 0x1f;
        } else if ((c & 0xf0) == 0xe0) {
            extra = 2;
            min = 0x800;
            c &= 0x0f;
        } else if ((c & 0xf8) == 0xf0) {
            extra = 3;
            min = 0x10000;
            c &= 0x07;
        } else {
            return false;
        }
        if (n - i <= extra) {
            return false;
        }
        for (size_t k = 1; k <= extra; k++) {
            if ((s[i + k] & 0xc0) != 0x80) {
                return false;
            }
            c = c << 6 | (s[i + k] & 0x3f);
        }
        if (c < min || c > 0x10ffff || (c >= 0xd800 && c <= 0xdfff) || refused_code_point(c)) {
            return false;
        }
        i += extra + 1;
    }
    return true;
}

/* ---- Per-peer backoff ---- */

static td_netlink_peer_t *find_peer(td_netlink_deck_t *deck, const uint8_t *peer, size_t len)
{
    for (size_t i = 0; i < TD_NETLINK_PEERS; i++) {
        td_netlink_peer_t *p = &deck->peers[i];
        if (p->used && p->len == len && memcmp(p->key, peer, len) == 0) {
            return p;
        }
    }
    return NULL;
}

uint32_t td_netlink_backoff_left(const td_netlink_deck_t *deck, const uint8_t *peer, size_t peer_len)
{
    const td_netlink_peer_t *p = find_peer((td_netlink_deck_t *)deck, peer, peer_len);
    if (!p || !before(deck, p->until_ms)) {
        return 0;
    }
    return p->until_ms - now_ms(deck);
}

static bool counted_end(td_netlink_end_t end)
{
    return end == TD_NETLINK_END_DEADLINE || end == TD_NETLINK_END_AUTH || end == TD_NETLINK_END_PROOF
           || end == TD_NETLINK_END_MALFORMED || end == TD_NETLINK_END_PEER_CLOSED;
}

static void backoff_end(td_netlink_deck_t *deck, const uint8_t *peer, size_t len, td_netlink_end_t end)
{
    td_netlink_peer_t *p = find_peer(deck, peer, len);
    if (end == TD_NETLINK_END_READY) {
        if (p) {
            memset(p, 0, sizeof *p);
        }
        return;
    }
    if (!counted_end(end) || len == 0) {
        return;
    }
    if (!p) {
        /* Keep at least TD_NETLINK_PEERS peers: a free slot, else the one whose last failure is oldest. */
        p = &deck->peers[0];
        for (size_t i = 0; i < TD_NETLINK_PEERS; i++) {
            td_netlink_peer_t *q = &deck->peers[i];
            if (!q->used) {
                p = q;
                break;
            }
            if ((int32_t)(q->last_ms - p->last_ms) < 0) {
                p = q;
            }
        }
        memset(p, 0, sizeof *p);
        p->used = true;
        p->len = (uint8_t)len;
        memcpy(p->key, peer, len);
    }
    p->failures++;
    const uint32_t shift = p->failures - 1 < 16 ? p->failures - 1 : 16;
    uint32_t delay = TD_NETLINK_BACKOFF_FIRST_MS << shift;
    if (delay > TD_NETLINK_BACKOFF_CAP_MS || shift >= 16) {
        delay = TD_NETLINK_BACKOFF_CAP_MS;
    }
    p->last_ms = now_ms(deck);
    p->until_ms = p->last_ms + delay;
}

/* ---- Pairing windows and the failure counter ---- */

static uint32_t lock_duration(uint32_t failures)
{
    const uint32_t shift = failures - TD_NETLINK_LOCK_AFTER;
    if (shift >= 5) {
        return TD_NETLINK_LOCK_CAP_MS;
    }
    const uint32_t ms = TD_NETLINK_LOCK_FIRST_MS << shift;
    return ms > TD_NETLINK_LOCK_CAP_MS ? TD_NETLINK_LOCK_CAP_MS : ms;
}

uint32_t td_netlink_lock_left(const td_netlink_deck_t *deck)
{
    if (!deck->locked || !before(deck, deck->lock_until_ms)) {
        return 0;
    }
    return deck->lock_until_ms - now_ms(deck);
}

static bool save_failures(td_netlink_deck_t *deck)
{
    deck->lock_saved_ms = now_ms(deck);
    return deck->env.save_failures(deck->env.ctx, deck->pair_failures, td_netlink_lock_left(deck));
}

static void window_end(td_netlink_deck_t *deck, td_netlink_reason_t why)
{
    td_netlink_window_t *w = &deck->window;
    if (!w->open) {
        return;
    }
    const uint32_t generation = w->generation;
    td_netlink_wipe(w, sizeof *w);
    w->ended = why;
    w->generation = generation + 1;
}

/* One more failure against pair_failures, committed before the caller answers. Three or more lock pairing and end the
 * window. False when the counter could not be stored, which ends the window too. */
static bool count_failure(td_netlink_deck_t *deck)
{
    deck->pair_failures++;
    if (deck->pair_failures >= TD_NETLINK_LOCK_AFTER) {
        deck->locked = true;
        deck->lock_until_ms = now_ms(deck) + lock_duration(deck->pair_failures);
    }
    const bool stored = save_failures(deck);
    if (!stored || deck->locked) {
        window_end(deck, TD_NETLINK_AUTH);
    }
    return stored;
}

static void window_expire(td_netlink_deck_t *deck)
{
    if (deck->window.open && since(deck, deck->window.opened_ms) >= TD_NETLINK_WINDOW_MS) {
        window_end(deck, TD_NETLINK_EXPIRED);
    }
}

static int find_pairing(const td_netlink_pairings_t *pairings, const uint8_t id[TD_NETLINK_COMPANION_ID_LEN])
{
    for (int i = 0; i < pairings->count; i++) {
        if (memcmp(pairings->entries[i].companion_id, id, TD_NETLINK_COMPANION_ID_LEN) == 0) {
            return i;
        }
    }
    return -1;
}

bool td_netlink_open_tag(const uint8_t ltk[TD_NETLINK_KEY_LEN], const uint8_t share[TD_NETLINK_SHARE_LEN],
                         uint8_t tag[TD_NETLINK_OPEN_TAG_LEN])
{
#ifdef TD_NETLINK_TEST
    extern void td_netlink_test_tag_computed(void);
    td_netlink_test_tag_computed();
#endif
    uint8_t key[TD_NETLINK_KEY_LEN], mac[TD_NETLINK_H_LEN];
    const td_netlink_part_t parts[] = { { "open", 4 }, { share, TD_NETLINK_SHARE_LEN } };
    const bool ok = td_netlink_derive(ltk, "session/open", key) && td_netlink_hmac(key, sizeof key, parts, 2, mac);
    memcpy(tag, mac, TD_NETLINK_OPEN_TAG_LEN);
    td_netlink_wipe(key, sizeof key);
    td_netlink_wipe(mac, sizeof mac);
    return ok;
}

/* Compute all four slots before deciding, hiding both the matching slot and the Pairing count. */
static int find_tagged(const td_netlink_pairings_t *pairings, const uint8_t tag[TD_NETLINK_OPEN_TAG_LEN],
                       const uint8_t share[TD_NETLINK_SHARE_LEN])
{
    static const uint8_t dummy[TD_NETLINK_KEY_LEN] = { 0 };
    int found = -1;
    for (int i = 0; i < TD_NETLINK_PAIRINGS_MAX; i++) {
        uint8_t mine[TD_NETLINK_OPEN_TAG_LEN];
        const bool occupied = i < pairings->count;
        const uint8_t *key = occupied ? pairings->entries[i].ltk : dummy;
        const bool match = td_netlink_open_tag(key, share, mine)
                           && td_netlink_equal(mine, tag, TD_NETLINK_OPEN_TAG_LEN);
        if (match && occupied && found < 0) {
            found = i;
        }
    }
    return found;
}

void td_netlink_deck_init(td_netlink_deck_t *deck, const uint8_t device_id[TD_NETLINK_DEVICE_ID_LEN],
                          const td_netlink_pairings_t *stored, uint32_t pair_failures, uint32_t lock_left_ms,
                          const td_netlink_env_t *env)
{
    memset(deck, 0, sizeof *deck);
    memcpy(deck->device_id, device_id, TD_NETLINK_DEVICE_ID_LEN);
    if (stored && stored->count <= TD_NETLINK_PAIRINGS_MAX) {
        deck->pairings = *stored;
    }
    deck->env = *env;
    deck->pair_failures = pair_failures;
    if (lock_left_ms > 0) {
        deck->locked = true;
        deck->lock_until_ms = now_ms(deck) + (lock_left_ms > TD_NETLINK_LOCK_CAP_MS ? TD_NETLINK_LOCK_CAP_MS
                                                                                     : lock_left_ms);
        deck->lock_saved_ms = now_ms(deck);
    }
}

td_netlink_reason_t td_netlink_window_open(td_netlink_deck_t *deck)
{
    td_netlink_window_t *w = &deck->window;
    window_expire(deck);
    if (td_netlink_lock_left(deck) > 0) {
        return TD_NETLINK_CLOSED;
    }
    if (w->open) {
        return TD_NETLINK_REASON_NONE;
    }
    if (deck->pairings.count >= TD_NETLINK_PAIRINGS_MAX) {
        return TD_NETLINK_FULL;
    }
    const uint32_t generation = w->generation;
    memset(w, 0, sizeof *w);
    w->generation = generation + 1;
    deck->env.rng(deck->env.ctx, w->window_id, sizeof w->window_id);
    w->opened_ms = now_ms(deck);
    w->open = true;
    return TD_NETLINK_REASON_NONE;
}

void td_netlink_window_cancel(td_netlink_deck_t *deck)
{
    window_end(deck, TD_NETLINK_CANCELLED);
}

uint32_t td_netlink_window_left(const td_netlink_deck_t *deck)
{
    const td_netlink_window_t *w = &deck->window;
    if (!w->open) {
        return 0;
    }
    const uint32_t open_for = since(deck, w->opened_ms);
    return open_for >= TD_NETLINK_WINDOW_MS ? 0 : TD_NETLINK_WINDOW_MS - open_for;
}

void td_netlink_deck_tick(td_netlink_deck_t *deck)
{
    window_expire(deck);
    if (!deck->locked) {
        return;
    }
    if (td_netlink_lock_left(deck) == 0) {
        deck->locked = false;
        save_failures(deck);
    } else if (since(deck, deck->lock_saved_ms) >= TD_NETLINK_LOCK_SAVE_MS) {
        save_failures(deck);
    }
}

bool td_netlink_forget(td_netlink_deck_t *deck)
{
    window_end(deck, TD_NETLINK_CANCELLED);
    const td_netlink_pairings_t empty = { 0 };
    const bool erased = deck->env.save_pairings(deck->env.ctx, &empty);
    td_netlink_wipe(&deck->pairings, sizeof deck->pairings);
    deck->pair_failures = 0;
    deck->locked = false;
    deck->lock_until_ms = 0;
    const bool reset = save_failures(deck);
    return erased && reset;
}

bool td_netlink_names_stored(const td_netlink_deck_t *deck, const uint8_t *body, size_t len)
{
    return len == 2 + OPEN_FIELDS && body[0] == TD_NETLINK_SESSION_OPEN && body[1] == TD_NETLINK_VERSION
           && find_tagged(&deck->pairings, body + 2, body + 2 + TD_NETLINK_OPEN_TAG_LEN) >= 0;
}

/* ---- Connections ---- */

void td_netlink_conn_init(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *peer, size_t peer_len)
{
    memset(conn, 0, sizeof *conn);
    conn->state = TD_NETLINK_CONN_FIRST;
    conn->started_ms = now_ms(deck);
    conn->peer_len = (uint8_t)(peer_len > TD_NETLINK_PEER_LEN ? TD_NETLINK_PEER_LEN : peer_len);
    memcpy(conn->peer, peer, conn->peer_len);
    conn->rx.closed = true;
    conn->tx.closed = true;
}

void td_netlink_conn_limits(const td_netlink_conn_t *conn, uint16_t *min, uint16_t *max)
{
    if (conn->state == TD_NETLINK_CONN_READY) {
        *min = TD_NETLINK_SEALED_MIN;
        *max = TD_NETLINK_SEALED_MAX;
    } else {
        *min = TD_NETLINK_HANDSHAKE_MIN;
        *max = TD_NETLINK_HANDSHAKE_MAX;
    }
}

bool td_netlink_conn_proven(const td_netlink_conn_t *conn)
{
    return conn->state == TD_NETLINK_CONN_READY;
}

bool td_netlink_conn_worn(const td_netlink_conn_t *conn)
{
    const uint64_t last = TD_NETLINK_COUNTER_LIMIT - TD_NETLINK_COUNTER_MARGIN;
    return conn->state == TD_NETLINK_CONN_READY && (conn->rx.counter > last || conn->tx.counter > last);
}

static bool pairing_state(const td_netlink_conn_t *conn)
{
    return conn->state >= TD_NETLINK_CONN_PAIR_CODE && conn->state <= TD_NETLINK_CONN_PAIR_COMMIT;
}

bool td_netlink_conn_bound(const td_netlink_conn_t *conn, const td_netlink_deck_t *deck)
{
    return pairing_state(conn) && deck->window.open && deck->window.generation == conn->hs.pair.window_generation;
}

void td_netlink_conn_end(td_netlink_conn_t *conn, td_netlink_deck_t *deck, td_netlink_end_t end)
{
    (void)deck;
    if (conn->end == TD_NETLINK_END_NONE) {
        conn->end = end;
    }
}

void td_netlink_conn_close(td_netlink_conn_t *conn, td_netlink_deck_t *deck, td_netlink_gone_t how)
{
    if (conn->state == TD_NETLINK_CONN_CLOSED && conn->end == TD_NETLINK_END_NONE) {
        conn->end = TD_NETLINK_END_LOCAL;
    }
    if (conn->end == TD_NETLINK_END_NONE) {
        conn->end = how == TD_NETLINK_GONE_PEER    ? TD_NETLINK_END_PEER_CLOSED
                    : how == TD_NETLINK_GONE_RADIO ? TD_NETLINK_END_RADIO_LOST
                                                   : TD_NETLINK_END_LOCAL;
    }
    if (td_netlink_conn_bound(conn, deck)) {
        /* A dropped window counts once, unless a person ended it, it expired, storage failed, it paired, or it ended on
         * the failed proof that pair_proof already counted (`auth`, as the reference's windowEnded has it). A window
         * dropped after an earlier failed proof returned to code entry still counts: each failure is its own. */
        const td_netlink_end_t end = conn->end;
        const bool free_end = end == TD_NETLINK_END_CANCELLED || end == TD_NETLINK_END_WINDOW_EXPIRED
                              || end == TD_NETLINK_END_STORAGE || end == TD_NETLINK_END_PAIRED
                              || end == TD_NETLINK_END_AUTH;
        if (!free_end) {
            count_failure(deck);
        }
        window_end(deck, TD_NETLINK_REASON_NONE);
    }
    backoff_end(deck, conn->peer, conn->peer_len, conn->end);
    td_netlink_dir_close(&conn->rx);
    td_netlink_dir_close(&conn->tx);
    td_netlink_wipe(&conn->hs, sizeof conn->hs);
    conn->state = TD_NETLINK_CONN_CLOSED;
}

/* ---- Admission ---- */

static bool owner_holds(const td_netlink_conn_t *owner, const td_netlink_deck_t *deck)
{
    return td_netlink_conn_proven(owner) || td_netlink_conn_bound(owner, deck);
}

static void error_out(td_netlink_out_t *out, td_netlink_reason_t reason)
{
    const uint8_t byte = (uint8_t)reason;
    const td_netlink_part_t fields[] = { { &byte, 1 } };
    put_item(out, TD_NETLINK_ERROR, fields, 1);
}

static td_netlink_waiter_t *waiter(td_netlink_deck_t *deck, int slot)
{
    return slot >= 0 && slot < TD_NETLINK_WAITERS && deck->waiters[slot].used ? &deck->waiters[slot] : NULL;
}

static void waiter_free(td_netlink_deck_t *deck, int slot, td_netlink_end_t end)
{
    td_netlink_waiter_t *w = waiter(deck, slot);
    if (w) {
        backoff_end(deck, w->peer, w->peer_len, end);
        memset(w, 0, sizeof *w);
    }
}

td_netlink_admit_t td_netlink_admit(td_netlink_deck_t *deck, const td_netlink_conn_t *owner, const uint8_t *peer,
                                    size_t peer_len, int *slot, int *evicted)
{
    *slot = -1;
    *evicted = -1;
    if (!owner) {
        return TD_NETLINK_ADMIT_OWNER;
    }
    if (owner_holds(owner, deck)) {
        return TD_NETLINK_ADMIT_BUSY;
    }
    /* Pushed out first: the oldest waiter that hasn't sent a byte, then the oldest from a peer in backoff, whose first
     * item could only take over if it carried a stored Pairing's open_tag. */
    int free_slot = -1, oldest = -1, oldest_backed_off = -1;
    for (int i = 0; i < TD_NETLINK_WAITERS; i++) {
        const td_netlink_waiter_t *w = &deck->waiters[i];
        if (!w->used) {
            if (free_slot < 0) {
                free_slot = i;
            }
        } else if (!w->started) {
            if (oldest < 0 || (int32_t)(w->since_ms - deck->waiters[oldest].since_ms) < 0) {
                oldest = i;
            }
        } else if (td_netlink_backoff_left(deck, w->peer, w->peer_len) > 0
                   && (oldest_backed_off < 0
                       || (int32_t)(w->since_ms - deck->waiters[oldest_backed_off].since_ms) < 0)) {
            oldest_backed_off = i;
        }
    }
    if (oldest < 0) {
        oldest = oldest_backed_off;
    }
    if (free_slot < 0) {
        if (oldest < 0) {
            return TD_NETLINK_ADMIT_BUSY;
        }
        /* Pushed out for a newer connection: the Device's decision, like a busy refusal, so it doesn't count. */
        *evicted = oldest;
        free_slot = oldest;
    }
    td_netlink_waiter_t *w = &deck->waiters[free_slot];
    memset(w, 0, sizeof *w);
    w->used = true;
    w->peer_len = (uint8_t)(peer_len > TD_NETLINK_PEER_LEN ? TD_NETLINK_PEER_LEN : peer_len);
    memcpy(w->peer, peer, w->peer_len);
    w->since_ms = now_ms(deck);
    *slot = free_slot;
    return TD_NETLINK_ADMIT_WAIT;
}

void td_netlink_waiter_started(td_netlink_deck_t *deck, int slot)
{
    td_netlink_waiter_t *w = waiter(deck, slot);
    if (w) {
        w->started = true;
    }
}

td_netlink_action_t td_netlink_waiter_item(td_netlink_deck_t *deck, const td_netlink_conn_t *owner, int slot,
                                           const uint8_t *body, size_t len, td_netlink_out_t *out)
{
    if (!waiter(deck, slot)) {
        return TD_NETLINK_CLOSE;
    }
    if (!owner || (!owner_holds(owner, deck) && td_netlink_names_stored(deck, body, len))) {
        /* The connection's own first-item checks, backoff among them, run when the caller feeds it the item. */
        memset(&deck->waiters[slot], 0, sizeof deck->waiters[slot]);
        return TD_NETLINK_NOTHING;
    }
    if (!owner_holds(owner, deck) && len >= 2 && body[0] == TD_NETLINK_SESSION_OPEN
        && td_netlink_backoff_left(deck, deck->waiters[slot].peer, deck->waiters[slot].peer_len) == 0) {
        /* A session.open whose tag matches no Pairing gets `auth` and takes nothing over. */
        error_out(out, TD_NETLINK_AUTH);
        waiter_free(deck, slot, TD_NETLINK_END_AUTH);
        return TD_NETLINK_SEND_CLOSE;
    }
    error_out(out, TD_NETLINK_BUSY);
    waiter_free(deck, slot, TD_NETLINK_END_BUSY);
    return TD_NETLINK_SEND_CLOSE;
}

td_netlink_action_t td_netlink_waiter_tick(td_netlink_deck_t *deck, const td_netlink_conn_t *owner, int slot,
                                           td_netlink_out_t *out)
{
    const td_netlink_waiter_t *w = waiter(deck, slot);
    if (!w) {
        return TD_NETLINK_NOTHING;
    }
    if (since(deck, w->since_ms) >= TD_NETLINK_FIRST_ITEM_MS) {
        error_out(out, TD_NETLINK_EXPIRED);
        waiter_free(deck, slot, TD_NETLINK_END_DEADLINE);
        return TD_NETLINK_SEND_CLOSE;
    }
    if (owner && owner_holds(owner, deck)) {
        error_out(out, TD_NETLINK_BUSY);
        waiter_free(deck, slot, TD_NETLINK_END_BUSY);
        return TD_NETLINK_SEND_CLOSE;
    }
    return TD_NETLINK_NOTHING;
}

void td_netlink_waiter_gone(td_netlink_deck_t *deck, int slot)
{
    waiter_free(deck, slot, TD_NETLINK_END_PEER_CLOSED);
}

void td_netlink_waiter_drop(td_netlink_deck_t *deck, int slot)
{
    waiter_free(deck, slot, TD_NETLINK_END_LOCAL);
}

/* Marks the end and stops taking items; the carrier's close does the accounting. */
static td_netlink_action_t stop(td_netlink_conn_t *conn, td_netlink_end_t end)
{
    if (conn->end == TD_NETLINK_END_NONE) {
        conn->end = end;
    }
    td_netlink_dir_close(&conn->rx);
    td_netlink_dir_close(&conn->tx);
    if (!pairing_state(conn)) {
        td_netlink_wipe(&conn->hs, sizeof conn->hs);
    }
    return TD_NETLINK_CLOSE;
}

static td_netlink_action_t fail(td_netlink_conn_t *conn, td_netlink_out_t *out, td_netlink_reason_t reason,
                                td_netlink_end_t end)
{
    const uint8_t byte = (uint8_t)reason;
    const td_netlink_part_t fields[] = { { &byte, 1 } };
    put_item(out, TD_NETLINK_ERROR, fields, 1);
    stop(conn, end);
    return TD_NETLINK_SEND_CLOSE;
}

static td_netlink_action_t malformed(td_netlink_conn_t *conn, td_netlink_out_t *out)
{
    return fail(conn, out, TD_NETLINK_ARGS, TD_NETLINK_END_MALFORMED);
}

td_netlink_action_t td_netlink_conn_bad_length(td_netlink_conn_t *conn, td_netlink_deck_t *deck,
                                               td_netlink_out_t *out)
{
    (void)deck;
    out->len = 0;
    if (conn->state == TD_NETLINK_CONN_READY || conn->state == TD_NETLINK_CONN_CLOSED) {
        return stop(conn, conn->state == TD_NETLINK_CONN_READY ? TD_NETLINK_END_READY : TD_NETLINK_END_MALFORMED);
    }
    return malformed(conn, out);
}

static td_netlink_action_t deadline(td_netlink_conn_t *conn, td_netlink_deck_t *deck, td_netlink_out_t *out)
{
    switch (conn->state) {
    case TD_NETLINK_CONN_FIRST:
        if (since(deck, conn->started_ms) >= TD_NETLINK_FIRST_ITEM_MS) {
            return fail(conn, out, TD_NETLINK_EXPIRED, TD_NETLINK_END_DEADLINE);
        }
        return TD_NETLINK_NOTHING;
    case TD_NETLINK_CONN_SESSION_PROOF:
        if (since(deck, conn->started_ms) >= TD_NETLINK_SETUP_MS) {
            return fail(conn, out, TD_NETLINK_EXPIRED, TD_NETLINK_END_DEADLINE);
        }
        return TD_NETLINK_NOTHING;
    case TD_NETLINK_CONN_PAIR_CODE:
    case TD_NETLINK_CONN_PAIR_PROOF:
    case TD_NETLINK_CONN_PAIR_COMMIT:
        window_expire(deck);
        if (!td_netlink_conn_bound(conn, deck)) {
            const td_netlink_reason_t why = deck->window.ended;
            if (why == TD_NETLINK_CANCELLED) {
                return fail(conn, out, TD_NETLINK_CANCELLED, TD_NETLINK_END_CANCELLED);
            }
            return fail(conn, out, TD_NETLINK_EXPIRED, TD_NETLINK_END_WINDOW_EXPIRED);
        }
        if (conn->hs.pair.attempt_started && since(deck, conn->hs.pair.attempt_ms) >= TD_NETLINK_ATTEMPT_MS) {
            return fail(conn, out, TD_NETLINK_EXPIRED, TD_NETLINK_END_DEADLINE);
        }
        return TD_NETLINK_NOTHING;
    case TD_NETLINK_CONN_READY: {
        const uint32_t last = conn->hello_seen ? conn->last_rx_ms : conn->ready_ms;
        const bool usb = deck->env.usb_online && deck->env.usb_online(deck->env.ctx);
        if (usb || since(deck, last) >= (conn->hello_seen ? TD_NETLINK_EXPIRY_MS : TD_NETLINK_HELLO_MS)) {
            return stop(conn, TD_NETLINK_END_READY);
        }
        return TD_NETLINK_NOTHING;
    }
    case TD_NETLINK_CONN_CLOSED:
        break;
    }
    return TD_NETLINK_NOTHING;
}

static void hex(char *out, const uint8_t *in, size_t len)
{
    static const char digits[] = "0123456789abcdef";
    for (size_t i = 0; i < len; i++) {
        out[2 * i] = digits[in[i] >> 4];
        out[2 * i + 1] = digits[in[i] & 15];
    }
}

static void draw_code(td_netlink_deck_t *deck, char code[TD_NETLINK_CODE_LEN])
{
    uint32_t r;
    do {
        uint8_t raw[4];
        deck->env.rng(deck->env.ctx, raw, sizeof raw);
        r = (uint32_t)raw[0] << 24 | (uint32_t)raw[1] << 16 | (uint32_t)raw[2] << 8 | raw[3];
        td_netlink_wipe(raw, sizeof raw);
    } while (r >= CODE_DRAW_LIMIT);
    r %= 1000000u;
    for (int i = TD_NETLINK_CODE_LEN - 1; i >= 0; i--) {
        code[i] = (char)('0' + r % 10);
        r /= 10;
    }
}

static td_netlink_action_t pair_request(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                        size_t len, td_netlink_out_t *out)
{
    const uint8_t *fields = body + 2;
    const size_t n = len - 2;
    if (n < REQUEST_MIN || n > REQUEST_MAX || fields[TD_NETLINK_COMPANION_ID_LEN] != n - TD_NETLINK_COMPANION_ID_LEN - 1
            || !td_netlink_name_valid(fields + TD_NETLINK_COMPANION_ID_LEN + 1, n - TD_NETLINK_COMPANION_ID_LEN - 1)) {
        return malformed(conn, out);
    }
    td_netlink_window_t *w = &deck->window;
    window_expire(deck);
    if (td_netlink_lock_left(deck) > 0 || !w->open) {
        return fail(conn, out, TD_NETLINK_CLOSED, TD_NETLINK_END_CLOSED);
    }
    if (deck->pairings.count >= TD_NETLINK_PAIRINGS_MAX || find_pairing(&deck->pairings, fields) >= 0) {
        return fail(conn, out, TD_NETLINK_FULL, TD_NETLINK_END_FULL);
    }
    if (w->bound) {
        return fail(conn, out, TD_NETLINK_BUSY, TD_NETLINK_END_BUSY);
    }
    w->bound = true;
    draw_code(deck, w->code);
    td_netlink_pair_state_t *p = &conn->hs.pair;
    p->window_generation = w->generation;
    memcpy(conn->companion_id, fields, TD_NETLINK_COMPANION_ID_LEN);
    conn->name_len = fields[TD_NETLINK_COMPANION_ID_LEN];
    memcpy(conn->name, fields + TD_NETLINK_COMPANION_ID_LEN + 1, conn->name_len);
    w->name_len = conn->name_len;
    memcpy(w->name, conn->name, conn->name_len);
    p->request[0] = (uint8_t)(len >> 8);
    p->request[1] = (uint8_t)len;
    memcpy(p->request + 2, body, len);
    p->request_len = (uint16_t)(len + 2);

    const uint32_t remaining = td_netlink_window_left(deck);
    const uint8_t left[4] = {
        (uint8_t)(remaining >> 24), (uint8_t)(remaining >> 16), (uint8_t)(remaining >> 8), (uint8_t)remaining,
    };
    const td_netlink_part_t offer[] = {
        { deck->device_id, TD_NETLINK_DEVICE_ID_LEN }, { w->window_id, sizeof w->window_id }, { left, 4 },
    };
    put_item(out, TD_NETLINK_PAIR_OFFER, offer, 3);
    memcpy(p->offer, out->item, out->len);
    conn->state = TD_NETLINK_CONN_PAIR_CODE;
    return TD_NETLINK_SEND;
}

static td_netlink_action_t pair_start(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                      size_t len, td_netlink_out_t *out)
{
    if (len != 2 + START_FIELDS) {
        return malformed(conn, out);
    }
    td_netlink_pair_state_t *p = &conn->hs.pair;
    if (!p->attempt_started) {
        p->attempt_started = true;
        p->attempt_ms = now_ms(deck);
    }
    uint8_t salt[TD_NETLINK_SALT_LEN], b[TD_NETLINK_EXP_LEN], nonce[32];
    deck->env.rng(deck->env.ctx, salt, sizeof salt);
    deck->env.rng(deck->env.ctx, b, sizeof b);
    deck->env.rng(deck->env.ctx, nonce, sizeof nonce);

    char identity[10 + 2 * TD_NETLINK_DEVICE_ID_LEN + 1 + 2 * TD_NETLINK_COMPANION_ID_LEN];
    memcpy(identity, "touchdeck:", 10);
    hex(identity + 10, deck->device_id, TD_NETLINK_DEVICE_ID_LEN);
    identity[10 + 2 * TD_NETLINK_DEVICE_ID_LEN] = ':';
    hex(identity + 11 + 2 * TD_NETLINK_DEVICE_ID_LEN, conn->companion_id, TD_NETLINK_COMPANION_ID_LEN);

    uint8_t B[TD_NETLINK_SRP_LEN], key[TD_NETLINK_H512_LEN];
    const td_netlink_srp_in_t in = {
        .identity = (const uint8_t *)identity, .identity_len = sizeof identity,
        .code = deck->window.code, .salt = salt, .b = b, .A = body + 2,
    };
    const td_netlink_srp_result_t srp = td_netlink_srp_server(&in, B, key);
    td_netlink_wipe(b, sizeof b);
    if (srp != TD_NETLINK_SRP_OK) {
        /* An invalid public value closes the carrier connection and the window. */
        return malformed(conn, out);
    }
    const td_netlink_part_t challenge[] = { { salt, sizeof salt }, { B, sizeof B }, { nonce, sizeof nonce } };
    put_item(out, TD_NETLINK_PAIR_CHALLENGE, challenge, 3);

    static const char label[] = "touchdeck/net-v2/pair";
    const uint8_t start_len[2] = { (uint8_t)(len >> 8), (uint8_t)len };
    const td_netlink_part_t transcript[] = {
        { label, sizeof label }, { p->request, p->request_len }, { p->offer, sizeof p->offer },
        { start_len, 2 }, { body, len }, { out->item, out->len },
    };
    uint8_t prk[TD_NETLINK_H_LEN], client_key[TD_NETLINK_KEY_LEN];
    const td_netlink_part_t t[] = { { p->transcript, TD_NETLINK_H_LEN } };
    const td_netlink_part_t ikm[] = { { key, sizeof key } };
    const bool ok = td_netlink_sha256(transcript, 6, p->transcript)
                    && td_netlink_hmac(p->transcript, TD_NETLINK_H_LEN, ikm, 1, prk)
                    && td_netlink_derive(prk, "pair/client", client_key)
                    && td_netlink_derive(prk, "pair/deck", p->deck_key)
                    && td_netlink_derive(prk, "pair/wrap", p->wrap_key)
                    && td_netlink_derive(prk, "pair/commit", p->commit_key)
                    && td_netlink_hmac(client_key, TD_NETLINK_KEY_LEN, t, 1, p->client_proof);
    td_netlink_wipe(key, sizeof key);
    td_netlink_wipe(prk, sizeof prk);
    td_netlink_wipe(client_key, sizeof client_key);
    if (!ok) {
        return malformed(conn, out);
    }
    conn->state = TD_NETLINK_CONN_PAIR_PROOF;
    const td_netlink_action_t late = deadline(conn, deck, out);
    return late == TD_NETLINK_NOTHING ? TD_NETLINK_SEND : late;
}

static void attempt_wipe(td_netlink_pair_state_t *p)
{
    td_netlink_wipe(p->transcript, sizeof p->transcript);
    td_netlink_wipe(p->client_proof, sizeof p->client_proof);
    td_netlink_wipe(p->deck_key, sizeof p->deck_key);
    td_netlink_wipe(p->wrap_key, sizeof p->wrap_key);
    td_netlink_wipe(p->commit_key, sizeof p->commit_key);
    td_netlink_wipe(p->ltk, sizeof p->ltk);
    p->attempt_started = false;
}

static td_netlink_action_t pair_proof(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                      size_t len, td_netlink_out_t *out)
{
    if (len != 2 + PROOF_FIELDS) {
        return malformed(conn, out);
    }
    td_netlink_pair_state_t *p = &conn->hs.pair;
    if (!td_netlink_equal(body + 2, p->client_proof, TD_NETLINK_H_LEN)) {
        attempt_wipe(p);
        deck->window.proofs++;
        const bool stored = count_failure(deck);
        if (!stored || deck->locked || deck->window.proofs >= TD_NETLINK_PROOFS_MAX) {
            window_end(deck, TD_NETLINK_AUTH);
            return fail(conn, out, TD_NETLINK_AUTH, TD_NETLINK_END_AUTH);
        }
        const uint8_t reason = TD_NETLINK_AUTH;
        const td_netlink_part_t fields[] = { { &reason, 1 } };
        put_item(out, TD_NETLINK_ERROR, fields, 1);
        conn->state = TD_NETLINK_CONN_PAIR_CODE;
        return TD_NETLINK_SEND;
    }
    deck->env.rng(deck->env.ctx, p->ltk, sizeof p->ltk);
    uint8_t device_proof[TD_NETLINK_H_LEN], wrapped[TD_NETLINK_KEY_LEN + TD_NETLINK_TAG_LEN];
    uint8_t aad[3 * TD_NETLINK_H_LEN];
    const td_netlink_part_t signed_part[] = { { p->transcript, TD_NETLINK_H_LEN }, { p->client_proof, TD_NETLINK_H_LEN } };
    const uint8_t zero_nonce[TD_NETLINK_NONCE_LEN] = { 0 };
    bool ok = td_netlink_hmac(p->deck_key, TD_NETLINK_KEY_LEN, signed_part, 2, device_proof);
    memcpy(aad, p->transcript, TD_NETLINK_H_LEN);
    memcpy(aad + TD_NETLINK_H_LEN, p->client_proof, TD_NETLINK_H_LEN);
    memcpy(aad + 2 * TD_NETLINK_H_LEN, device_proof, TD_NETLINK_H_LEN);
    ok = ok && td_netlink_seal(p->wrap_key, zero_nonce, aad, sizeof aad, p->ltk, TD_NETLINK_KEY_LEN, wrapped);
    td_netlink_wipe(p->deck_key, sizeof p->deck_key);
    td_netlink_wipe(p->wrap_key, sizeof p->wrap_key);
    if (!ok) {
        return malformed(conn, out);
    }
    const td_netlink_part_t accept[] = { { device_proof, sizeof device_proof }, { wrapped, sizeof wrapped } };
    put_item(out, TD_NETLINK_PAIR_ACCEPT, accept, 2);
    conn->state = TD_NETLINK_CONN_PAIR_COMMIT;
    return TD_NETLINK_SEND;
}

static bool commit_mac(const td_netlink_pair_state_t *p, const char *word, uint8_t out[TD_NETLINK_H_LEN])
{
    uint8_t digest[TD_NETLINK_H_LEN];
    const td_netlink_part_t ltk[] = { { p->ltk, TD_NETLINK_KEY_LEN } };
    bool ok = td_netlink_sha256(ltk, 1, digest);
    const td_netlink_part_t parts[] = { { word, 6 }, { p->transcript, TD_NETLINK_H_LEN }, { digest, sizeof digest } };
    ok = ok && td_netlink_hmac(p->commit_key, TD_NETLINK_KEY_LEN, parts, 3, out);
    td_netlink_wipe(digest, sizeof digest);
    return ok;
}

static td_netlink_action_t pair_commit(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                       size_t len, td_netlink_out_t *out)
{
    if (len != 2 + PROOF_FIELDS) {
        return malformed(conn, out);
    }
    td_netlink_pair_state_t *p = &conn->hs.pair;
    uint8_t expected[TD_NETLINK_H_LEN];
    if (!commit_mac(p, "commit", expected) || !td_netlink_equal(body + 2, expected, TD_NETLINK_H_LEN)) {
        return stop(conn, TD_NETLINK_END_PROOF);
    }
    const td_netlink_action_t late = deadline(conn, deck, out);
    if (late != TD_NETLINK_NOTHING) {
        return late;
    }
    if (deck->pairings.count >= TD_NETLINK_PAIRINGS_MAX || find_pairing(&deck->pairings, conn->companion_id) >= 0) {
        return fail(conn, out, TD_NETLINK_FULL, TD_NETLINK_END_FULL);
    }
    td_netlink_pairings_t next = deck->pairings;
    td_netlink_pairing_t *entry = &next.entries[next.count++];
    memset(entry, 0, sizeof *entry);
    memcpy(entry->companion_id, conn->companion_id, TD_NETLINK_COMPANION_ID_LEN);
    entry->name_len = conn->name_len;
    memcpy(entry->name, conn->name, conn->name_len);
    memcpy(entry->ltk, p->ltk, TD_NETLINK_KEY_LEN);
    const bool stored = deck->env.save_pairings(deck->env.ctx, &next);
    if (stored) {
        deck->pairings = next;
    }
    td_netlink_wipe(&next, sizeof next);
    uint8_t proof[TD_NETLINK_H_LEN];
    if (!stored || !commit_mac(p, "stored", proof)) {
        return fail(conn, out, TD_NETLINK_STORAGE, TD_NETLINK_END_STORAGE);
    }
    /* A committed Pairing resets pair_failures and clears the lockout. */
    deck->pair_failures = 0;
    deck->locked = false;
    save_failures(deck);
    const td_netlink_part_t fields[] = { { proof, sizeof proof } };
    put_item(out, TD_NETLINK_PAIR_STORED, fields, 1);
    conn->end = TD_NETLINK_END_PAIRED;
    window_end(deck, TD_NETLINK_REASON_NONE);
    stop(conn, TD_NETLINK_END_PAIRED);
    return TD_NETLINK_SEND_CLOSE;
}

static td_netlink_action_t session_open(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                        size_t len, td_netlink_out_t *out)
{
    if (len != 2 + OPEN_FIELDS) {
        return malformed(conn, out);
    }
    /* session.open names no id: its tag, made with one Pairing's LTK for this share, finds the Pairing. */
    const uint8_t *client_share = body + 2 + TD_NETLINK_OPEN_TAG_LEN;
    const int index = find_tagged(&deck->pairings, body + 2, client_share);
    if (index < 0) {
        return fail(conn, out, TD_NETLINK_AUTH, TD_NETLINK_END_AUTH);
    }
    if (deck->env.usb_online && deck->env.usb_online(deck->env.ctx)) {
        return fail(conn, out, TD_NETLINK_BUSY, TD_NETLINK_END_USB_ONLINE);
    }
    const td_netlink_pairing_t *pairing = &deck->pairings.entries[index];
    memcpy(conn->companion_id, pairing->companion_id, TD_NETLINK_COMPANION_ID_LEN);
    conn->name_len = pairing->name_len;
    memcpy(conn->name, pairing->name, pairing->name_len);

    /* A share that isn't canonical, that PSA refuses or that gives an all-zero Z closes with no error item. */
    uint8_t secret[TD_NETLINK_SHARE_LEN], device_share[TD_NETLINK_SHARE_LEN], z[TD_NETLINK_SHARE_LEN];
    if (!td_netlink_share_canonical(client_share)) {
        return stop(conn, TD_NETLINK_END_MALFORMED);
    }
    deck->env.rng(deck->env.ctx, secret, sizeof secret);
    const bool agreed = td_netlink_x25519_share(secret, device_share) && td_netlink_x25519(secret, client_share, z);
    td_netlink_wipe(secret, sizeof secret);
    if (!agreed) {
        td_netlink_wipe(z, sizeof z);
        return stop(conn, TD_NETLINK_END_MALFORMED);
    }

    td_netlink_session_state_t *k = &conn->hs.session;
    static const char label[] = "touchdeck/net-v2/session";
    const td_netlink_part_t transcript[] = {
        { label, sizeof label }, { deck->device_id, TD_NETLINK_DEVICE_ID_LEN },
        { pairing->companion_id, TD_NETLINK_COMPANION_ID_LEN },
        { client_share, TD_NETLINK_SHARE_LEN }, { device_share, TD_NETLINK_SHARE_LEN },
    };
    const td_netlink_part_t ikm[] = { { pairing->ltk, TD_NETLINK_KEY_LEN }, { z, sizeof z } };
    const td_netlink_part_t t[] = { { k->transcript, TD_NETLINK_H_LEN } };
    uint8_t prk[TD_NETLINK_H_LEN], deck_key[TD_NETLINK_KEY_LEN];
    const bool ok = td_netlink_sha256(transcript, 5, k->transcript)
                    && td_netlink_hmac(k->transcript, TD_NETLINK_H_LEN, ikm, 2, prk)
                    && td_netlink_derive(prk, "session/deck", deck_key)
                    && td_netlink_derive(prk, "session/client", k->client_key)
                    && td_netlink_derive(prk, "session/ready", k->ready_key)
                    && td_netlink_derive(prk, "session/c2d", k->c2d)
                    && td_netlink_derive(prk, "session/d2c", k->d2c)
                    && td_netlink_hmac(deck_key, TD_NETLINK_KEY_LEN, t, 1, k->deck_proof);
    td_netlink_wipe(z, sizeof z);
    td_netlink_wipe(prk, sizeof prk);
    td_netlink_wipe(deck_key, sizeof deck_key);
    if (!ok) {
        return stop(conn, TD_NETLINK_END_MALFORMED);
    }
    const td_netlink_part_t challenge[] = { { device_share, sizeof device_share }, { k->deck_proof, TD_NETLINK_H_LEN } };
    put_item(out, TD_NETLINK_SESSION_CHALLENGE, challenge, 2);
    conn->state = TD_NETLINK_CONN_SESSION_PROOF;
    return TD_NETLINK_SEND;
}

static td_netlink_action_t session_proof(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                         size_t len, td_netlink_out_t *out)
{
    if (len != 2 + PROOF_FIELDS) {
        return malformed(conn, out);
    }
    td_netlink_session_state_t *k = &conn->hs.session;
    uint8_t expected[TD_NETLINK_H_LEN], ready[TD_NETLINK_H_LEN];
    const td_netlink_part_t signed_part[] = { { k->transcript, TD_NETLINK_H_LEN }, { k->deck_proof, TD_NETLINK_H_LEN } };
    if (!td_netlink_hmac(k->client_key, TD_NETLINK_KEY_LEN, signed_part, 2, expected)
            || !td_netlink_equal(body + 2, expected, TD_NETLINK_H_LEN)) {
        return stop(conn, TD_NETLINK_END_PROOF);
    }
    const td_netlink_part_t ready_part[] = { { k->transcript, TD_NETLINK_H_LEN }, { body + 2, TD_NETLINK_H_LEN } };
    if (!td_netlink_hmac(k->ready_key, TD_NETLINK_KEY_LEN, ready_part, 2, ready)) {
        return stop(conn, TD_NETLINK_END_PROOF);
    }
    const td_netlink_part_t fields[] = { { ready, sizeof ready } };
    put_item(out, TD_NETLINK_SESSION_READY, fields, 1);
    td_netlink_dir_init(&conn->rx, TD_NETLINK_C2D, k->c2d);
    td_netlink_dir_init(&conn->tx, TD_NETLINK_D2C, k->d2c);
    td_netlink_wipe(&conn->hs, sizeof conn->hs);
    conn->ready_ms = now_ms(deck);
    conn->last_rx_ms = conn->ready_ms;
    conn->hello_seen = false;
    conn->state = TD_NETLINK_CONN_READY;
    /* Reaching session.ready resets this peer's backoff, and however the session ends later, nothing counts against
     * the peer: its end stays READY. */
    conn->end = TD_NETLINK_END_READY;
    backoff_end(deck, conn->peer, conn->peer_len, TD_NETLINK_END_READY);
    return TD_NETLINK_SEND;
}

void td_netlink_conn_item_started(td_netlink_conn_t *conn, td_netlink_deck_t *deck)
{
    if (conn->state == TD_NETLINK_CONN_PAIR_CODE && !conn->hs.pair.attempt_started) {
        conn->hs.pair.attempt_started = true;
        conn->hs.pair.attempt_ms = now_ms(deck);
    }
}

td_netlink_action_t td_netlink_conn_item(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *body,
                                         size_t len, td_netlink_out_t *out)
{
    out->len = 0;
    if (conn->state == TD_NETLINK_CONN_READY || conn->state == TD_NETLINK_CONN_CLOSED) {
        return stop(conn, conn->state == TD_NETLINK_CONN_READY ? TD_NETLINK_END_READY : TD_NETLINK_END_MALFORMED);
    }
    const td_netlink_action_t late = deadline(conn, deck, out);
    if (late != TD_NETLINK_NOTHING) {
        return late;
    }
    if (len < TD_NETLINK_HANDSHAKE_MIN || len > TD_NETLINK_HANDSHAKE_MAX) {
        return malformed(conn, out);
    }
    if (body[1] != TD_NETLINK_VERSION) {
        return fail(conn, out, TD_NETLINK_BAD_VERSION, TD_NETLINK_END_MALFORMED);
    }
    switch (conn->state) {
    case TD_NETLINK_CONN_FIRST:
        /* Per-peer backoff applies to the first item. A session.open with a stored Pairing's tag skips it. */
        if (td_netlink_backoff_left(deck, conn->peer, conn->peer_len) > 0 && !td_netlink_names_stored(deck, body, len)) {
            return fail(conn, out, TD_NETLINK_BUSY, TD_NETLINK_END_BUSY);
        }
        if (body[0] == TD_NETLINK_PAIR_REQUEST) {
            return pair_request(conn, deck, body, len, out);
        }
        if (body[0] == TD_NETLINK_SESSION_OPEN) {
            return session_open(conn, deck, body, len, out);
        }
        break;
    case TD_NETLINK_CONN_PAIR_CODE:
        if (body[0] == TD_NETLINK_PAIR_START) {
            return pair_start(conn, deck, body, len, out);
        }
        break;
    case TD_NETLINK_CONN_PAIR_PROOF:
        if (body[0] == TD_NETLINK_PAIR_PROOF) {
            return pair_proof(conn, deck, body, len, out);
        }
        break;
    case TD_NETLINK_CONN_PAIR_COMMIT:
        if (body[0] == TD_NETLINK_PAIR_COMMIT) {
            return pair_commit(conn, deck, body, len, out);
        }
        break;
    case TD_NETLINK_CONN_SESSION_PROOF:
        if (body[0] == TD_NETLINK_SESSION_PROOF) {
            return session_proof(conn, deck, body, len, out);
        }
        break;
    default:
        break;
    }
    return malformed(conn, out);
}

td_netlink_action_t td_netlink_conn_tick(td_netlink_conn_t *conn, td_netlink_deck_t *deck, td_netlink_out_t *out)
{
    out->len = 0;
    return deadline(conn, deck, out);
}

bool td_netlink_conn_record(td_netlink_conn_t *conn, td_netlink_deck_t *deck, const uint8_t *sealed, size_t len,
                            uint8_t *body)
{
    if (conn->state != TD_NETLINK_CONN_READY || !td_netlink_record_open(&conn->rx, sealed, len, body)) {
        stop(conn, TD_NETLINK_END_READY);
        return false;
    }
    conn->last_rx_ms = now_ms(deck);
    return true;
}

bool td_netlink_conn_send(td_netlink_conn_t *conn, const uint8_t *body, size_t len, uint8_t *wire)
{
    if (conn->state != TD_NETLINK_CONN_READY || !td_netlink_record_seal(&conn->tx, body, len, wire)) {
        stop(conn, TD_NETLINK_END_READY);
        return false;
    }
    return true;
}

void td_netlink_conn_hello(td_netlink_conn_t *conn)
{
    conn->hello_seen = true;
}
