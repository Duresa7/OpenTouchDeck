/*
 * net-v2's Device side on the host. The production td_netlink_core runs over ESP-IDF's TF-PSA-Crypto, with a scripted
 * random source, clock and store, against every vector in schema/protocol/net-v2-vectors.json (argv[1]) and against
 * the pairing, lockout, backoff and record rules of schema/protocol/net-v2.md. Every key and code here is a public
 * synthetic fixture.
 */
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "cJSON.h"
#include "td_netlink.h"
#include "td_netlink_ble.h"

static int g_failures;
static int g_checks;

#define CHECK(cond)                                                                                  \
    do {                                                                                             \
        g_checks++;                                                                                  \
        if (!(cond)) {                                                                               \
            fprintf(stderr, "%s:%d: %s: failed: %s\n", __FILE__, __LINE__, __func__, #cond);         \
            g_failures++;                                                                            \
        }                                                                                            \
    } while (0)

/* ---- Vectors ---- */

static cJSON *g_vectors;

static const cJSON *at(const cJSON *node, const char *path)
{
    char key[64];
    while (node && *path) {
        size_t n = strcspn(path, ".");
        assert(n < sizeof key);
        memcpy(key, path, n);
        key[n] = '\0';
        node = cJSON_GetObjectItemCaseSensitive(node, key);
        path += n + (path[n] == '.');
    }
    if (!node) {
        fprintf(stderr, "missing vector %s\n", path);
        exit(2);
    }
    return node;
}

static size_t unhex(const char *s, uint8_t *out, size_t cap)
{
    size_t n = strlen(s) / 2;
    assert(n <= cap);
    for (size_t i = 0; i < n; i++) {
        unsigned v;
        sscanf(s + 2 * i, "%2x", &v);
        out[i] = (uint8_t)v;
    }
    return n;
}

/* The bytes of a hex string vector. */
static size_t vec_of(const cJSON *node, const char *path, uint8_t *out, size_t cap)
{
    const cJSON *item = at(node, path);
    assert(cJSON_IsString(item));
    return unhex(item->valuestring, out, cap);
}

static size_t vec(const char *path, uint8_t *out, size_t cap)
{
    return vec_of(g_vectors, path, out, cap);
}

#define VEC(name, path) \
    uint8_t name[1100]; \
    const size_t name##_len = vec(path, name, sizeof name); \
    (void)name##_len

/* ---- The environment: a scripted random source, a clock and a store ---- */

typedef struct {
    uint8_t data[64];
    size_t len;
} chunk_t;

static chunk_t g_rng[16];
static size_t g_rng_head, g_rng_count;
static uint8_t g_rng_fill;
static uint32_t g_now;
static bool g_usb;
static bool g_pairings_fail, g_failures_fail;
static td_netlink_pairings_t g_saved;
static int g_pairing_saves;
static uint32_t g_saved_failures, g_saved_lock;
static int g_failure_saves;

static void rng_push(const uint8_t *data, size_t len)
{
    assert(g_rng_count < 16 && len <= 64);
    memcpy(g_rng[g_rng_count].data, data, len);
    g_rng[g_rng_count++].len = len;
}

static void rng_push_vec(const char *path)
{
    uint8_t buf[64];
    rng_push(buf, vec(path, buf, sizeof buf));
}

static void rng(void *ctx, uint8_t *out, size_t len)
{
    (void)ctx;
    if (g_rng_head < g_rng_count) {
        const chunk_t *c = &g_rng[g_rng_head++];
        CHECK(c->len == len);
        memcpy(out, c->data, len < c->len ? len : c->len);
        return;
    }
    for (size_t i = 0; i < len; i++) {
        out[i] = ++g_rng_fill;
    }
}

static uint32_t clock_ms(void *ctx)
{
    (void)ctx;
    return g_now;
}

static bool save_pairings(void *ctx, const td_netlink_pairings_t *pairings)
{
    (void)ctx;
    g_pairing_saves++;
    if (g_pairings_fail) {
        return false;
    }
    g_saved = *pairings;
    return true;
}

static bool save_failures(void *ctx, uint32_t failures, uint32_t lock_left_ms)
{
    (void)ctx;
    g_failure_saves++;
    if (g_failures_fail) {
        return false;
    }
    g_saved_failures = failures;
    g_saved_lock = lock_left_ms;
    return true;
}

static bool usb_online(void *ctx)
{
    (void)ctx;
    return g_usb;
}

static const td_netlink_env_t g_env = {
    .rng = rng, .clock = clock_ms, .save_pairings = save_pairings, .save_failures = save_failures,
    .usb_online = usb_online,
};

static td_netlink_deck_t g_deck;
static td_netlink_conn_t g_conn;
static td_netlink_out_t g_out;
static const uint8_t PEER_A[4] = { 192, 0, 2, 10 };
static const uint8_t PEER_B[4] = { 192, 0, 2, 11 };

static void reset_env(void)
{
    g_rng_head = g_rng_count = 0;
    g_rng_fill = 0;
    g_now = 1000000;
    g_usb = false;
    g_pairings_fail = g_failures_fail = false;
    memset(&g_saved, 0, sizeof g_saved);
    g_pairing_saves = g_failure_saves = 0;
    g_saved_failures = g_saved_lock = 0;
}

static td_netlink_pairing_t vector_pairing(void)
{
    td_netlink_pairing_t p = { 0 };
    vec("inputs.companionId", p.companion_id, sizeof p.companion_id);
    vec("inputs.ltk", p.ltk, sizeof p.ltk);
    const char *name = at(g_vectors, "inputs.companionName")->valuestring;
    p.name_len = (uint8_t)strlen(name);
    memcpy(p.name, name, p.name_len);
    return p;
}

/* A Device with the fixture device_id, and the fixture Pairing when `paired`. */
static void fresh_deck(bool paired, uint32_t failures, uint32_t lock_left)
{
    reset_env();
    uint8_t id[6];
    vec("inputs.deviceId", id, sizeof id);
    td_netlink_pairings_t stored = { 0 };
    if (paired) {
        stored.count = 1;
        stored.entries[0] = vector_pairing();
    }
    td_netlink_deck_init(&g_deck, id, &stored, failures, lock_left, &g_env);
}

static void connect_from(td_netlink_conn_t *conn, const uint8_t *peer)
{
    td_netlink_conn_init(conn, &g_deck, peer, 4);
}

/* One whole item, as a carrier reads it: the length checked against the state's bounds first. */
static td_netlink_action_t feed(td_netlink_conn_t *conn, const uint8_t *wire, size_t len)
{
    assert(len >= 2);
    const uint16_t body = (uint16_t)(wire[0] << 8 | wire[1]);
    uint16_t min, max;
    td_netlink_conn_limits(conn, &min, &max);
    if (body < min || body > max) {
        return td_netlink_conn_bad_length(conn, &g_deck, &g_out);
    }
    assert(body + 2u == len);
    td_netlink_conn_item_started(conn, &g_deck);
    return td_netlink_conn_item(conn, &g_deck, wire + 2, body, &g_out);
}

static td_netlink_action_t feed_vec(td_netlink_conn_t *conn, const char *path)
{
    VEC(wire, path);
    return feed(conn, wire, wire_len);
}

static bool out_is(const char *path)
{
    VEC(want, path);
    return g_out.len == want_len && memcmp(g_out.item, want, want_len) == 0;
}

static bool out_is_error(td_netlink_reason_t reason)
{
    return g_out.len == 5 && g_out.item[0] == 0 && g_out.item[1] == 3 && g_out.item[2] == TD_NETLINK_ERROR
           && g_out.item[3] == TD_NETLINK_VERSION && g_out.item[4] == reason;
}

/* The code bytes that draw 123456, the fixture code. */
static void push_code(void)
{
    static const uint8_t code[4] = { 0x00, 0x01, 0xe2, 0x40 };
    rng_push(code, sizeof code);
}

/* A window opened with the fixture window_id, bound by the fixture pair.request from `peer`. */
static void bind_window(td_netlink_conn_t *conn, const uint8_t *peer)
{
    rng_push_vec("inputs.windowId");
    CHECK(td_netlink_window_open(&g_deck) == TD_NETLINK_REASON_NONE);
    connect_from(conn, peer);
    push_code();
    CHECK(feed_vec(conn, "pairing.wire.request") == TD_NETLINK_SEND);
}

/* pair.start with the fixture salt, b and nonce, which gives the fixture challenge. */
static td_netlink_action_t start_attempt(td_netlink_conn_t *conn)
{
    rng_push_vec("inputs.salt");
    rng_push_vec("inputs.b");
    rng_push_vec("inputs.deckNonce");
    return feed_vec(conn, "pairing.wire.start");
}

static void open_session(td_netlink_conn_t *conn, const uint8_t *peer)
{
    connect_from(conn, peer);
    rng_push_vec("inputs.deviceEphemeral");
    CHECK(feed_vec(conn, "session.wire.open") == TD_NETLINK_SEND);
    CHECK(out_is("session.wire.challenge"));
    CHECK(feed_vec(conn, "session.wire.proof") == TD_NETLINK_SEND);
    CHECK(out_is("session.wire.ready"));
    CHECK(conn->state == TD_NETLINK_CONN_READY);
}

/* ---- Primitives ---- */

static void test_primitives_match_vectors(void)
{
    VEC(K, "pake.K_srp");
    VEC(transcript, "pairing.transcript");
    uint8_t prk[32], key[32], proof[32];
    const td_netlink_part_t ikm[] = { { K, K_len } };
    CHECK(td_netlink_hmac(transcript, 32, ikm, 1, prk));
    VEC(want_prk, "pairing.prk");
    CHECK(!memcmp(prk, want_prk, 32));
    static const char *const labels[][2] = {
        { "pair/client", "pairing.client" }, { "pair/deck", "pairing.deck" },
        { "pair/wrap", "pairing.wrap" }, { "pair/commit", "pairing.commit" },
    };
    for (size_t i = 0; i < 4; i++) {
        VEC(want, labels[i][1]);
        CHECK(td_netlink_derive(prk, labels[i][0], key) && !memcmp(key, want, 32));
    }
    VEC(client, "pairing.client");
    const td_netlink_part_t t[] = { { transcript, 32 } };
    VEC(want_proof, "pairing.clientProof");
    CHECK(td_netlink_hmac(client, 32, t, 1, proof) && !memcmp(proof, want_proof, 32));

    VEC(client_secret, "inputs.clientEphemeral");
    VEC(device_secret, "inputs.deviceEphemeral");
    VEC(client_share, "session.client_share");
    VEC(device_share, "session.device_share");
    VEC(Z, "session.Z");
    uint8_t share[32], z[32];
    CHECK(td_netlink_x25519_share(client_secret, share) && !memcmp(share, client_share, 32));
    CHECK(td_netlink_x25519_share(device_secret, share) && !memcmp(share, device_share, 32));
    CHECK(td_netlink_x25519(device_secret, client_share, z) && !memcmp(z, Z, 32));
    CHECK(td_netlink_x25519(client_secret, device_share, z) && !memcmp(z, Z, 32));
    VEC(session_prk, "session.prk");
    VEC(session_transcript, "session.transcript");
    VEC(ltk, "inputs.ltk");
    const td_netlink_part_t session_ikm[] = { { ltk, 32 }, { Z, 32 } };
    CHECK(td_netlink_hmac(session_transcript, 32, session_ikm, 2, prk) && !memcmp(prk, session_prk, 32));
    static const char *const session_labels[][2] = {
        { "session/deck", "session.deck" }, { "session/client", "session.client" },
        { "session/ready", "session.ready" }, { "session/c2d", "session.c2d" }, { "session/d2c", "session.d2c" },
    };
    for (size_t i = 0; i < 5; i++) {
        VEC(want, session_labels[i][1]);
        CHECK(td_netlink_derive(prk, session_labels[i][0], key) && !memcmp(key, want, 32));
    }
}

/* ---- Pairing ---- */

/* The Device answers the fixture Companion with every fixture pairing item, and stores the Pairing. */
static void test_pairing_vectors(void)
{
    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    CHECK(out_is("pairing.wire.offer"));
    CHECK(!memcmp(g_deck.window.code, at(g_vectors, "inputs.code")->valuestring, 6));
    CHECK(td_netlink_conn_bound(&g_conn, &g_deck) && !td_netlink_conn_proven(&g_conn));
    CHECK(start_attempt(&g_conn) == TD_NETLINK_SEND);
    CHECK(out_is("pairing.wire.challenge"));
    rng_push_vec("inputs.ltk");
    CHECK(feed_vec(&g_conn, "pairing.wire.proof") == TD_NETLINK_SEND);
    CHECK(out_is("pairing.wire.accept"));
    CHECK(feed_vec(&g_conn, "pairing.wire.commit") == TD_NETLINK_SEND_CLOSE);
    CHECK(out_is("pairing.wire.stored"));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    const td_netlink_pairing_t want = vector_pairing();
    CHECK(g_saved.count == 1 && !memcmp(&g_saved.entries[0], &want, sizeof want));
    CHECK(g_deck.pairings.count == 1 && !g_deck.window.open);
    CHECK(g_deck.pair_failures == 0 && g_saved_failures == 0);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_A, 4) == 0);
}

/* A wrong code gets `auth`, counts in NVS before the answer, and returns to code entry; the right code still pairs. */
static void test_wrong_code_then_right(void)
{
    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    CHECK(start_attempt(&g_conn) == TD_NETLINK_SEND);
    CHECK(feed_vec(&g_conn, "negative.wrong_code.proof") == TD_NETLINK_SEND);
    CHECK(out_is("negative.wrong_code.error"));
    CHECK(!strcmp(at(g_vectors, "negative.wrong_code.expected")->valuestring, "auth"));
    CHECK(g_saved_failures == 1 && g_deck.pair_failures == 1 && td_netlink_lock_left(&g_deck) == 0);
    CHECK(g_conn.state == TD_NETLINK_CONN_PAIR_CODE && td_netlink_conn_bound(&g_conn, &g_deck));
    CHECK(start_attempt(&g_conn) == TD_NETLINK_SEND);
    CHECK(out_is("pairing.wire.challenge"));
    rng_push_vec("inputs.ltk");
    CHECK(feed_vec(&g_conn, "pairing.wire.proof") == TD_NETLINK_SEND);
    CHECK(out_is("pairing.wire.accept"));
    CHECK(feed_vec(&g_conn, "pairing.wire.commit") == TD_NETLINK_SEND_CLOSE);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_deck.pairings.count == 1 && g_deck.pair_failures == 0 && g_saved_failures == 0);
}

/* A storage failure on commit answers `storage`, stores nothing, and doesn't count. */
static void test_commit_storage_failure(void)
{
    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    start_attempt(&g_conn);
    rng_push_vec("inputs.ltk");
    feed_vec(&g_conn, "pairing.wire.proof");
    g_pairings_fail = true;
    CHECK(feed_vec(&g_conn, "pairing.wire.commit") == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_STORAGE));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_deck.pairings.count == 0 && g_deck.pair_failures == 0 && !g_deck.window.open);
}

/* Three failed codes lock pairing for 30 s, then 60 s for the fourth, and the lock answers `closed`. */
static void test_lockout(void)
{
    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    for (int i = 0; i < 3; i++) {
        start_attempt(&g_conn);
        const td_netlink_action_t action = feed_vec(&g_conn, "negative.wrong_code.proof");
        CHECK(out_is_error(TD_NETLINK_AUTH));
        CHECK(action == (i < 2 ? TD_NETLINK_SEND : TD_NETLINK_SEND_CLOSE));
    }
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_deck.pair_failures == 3 && g_saved_failures == 3 && g_saved_lock == 30000);
    CHECK(!g_deck.window.open && td_netlink_lock_left(&g_deck) == 30000);
    CHECK(td_netlink_window_open(&g_deck) == TD_NETLINK_CLOSED && !g_deck.window.open);
    connect_from(&g_conn, PEER_B);
    CHECK(feed_vec(&g_conn, "pairing.wire.request") == TD_NETLINK_SEND_CLOSE && out_is("errors.closed"));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);

    g_now += 30000;
    td_netlink_deck_tick(&g_deck);
    CHECK(td_netlink_lock_left(&g_deck) == 0 && g_saved_lock == 0 && g_saved_failures == 3);
    rng_push_vec("inputs.windowId");
    CHECK(td_netlink_window_open(&g_deck) == TD_NETLINK_REASON_NONE);
    connect_from(&g_conn, PEER_B);
    push_code();
    CHECK(feed_vec(&g_conn, "pairing.wire.request") == TD_NETLINK_SEND);
    start_attempt(&g_conn);
    CHECK(feed_vec(&g_conn, "negative.wrong_code.proof") == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_AUTH));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_deck.pair_failures == 4 && g_saved_lock == 60000);

    /* The lock is capped at 15 minutes. */
    fresh_deck(false, 20, 0);
    g_deck.window.open = false;
    rng_push_vec("inputs.windowId");
    td_netlink_window_open(&g_deck);
    connect_from(&g_conn, PEER_A);
    push_code();
    feed_vec(&g_conn, "pairing.wire.request");
    start_attempt(&g_conn);
    feed_vec(&g_conn, "negative.wrong_code.proof");
    CHECK(g_saved_failures == 21 && g_saved_lock == TD_NETLINK_LOCK_CAP_MS);
}

/* A lockout resumes from NVS after a reboot, is saved at least once a minute while it runs, and ends. */
static void test_lockout_survives_reboot(void)
{
    fresh_deck(false, 5, 120000);
    CHECK(td_netlink_lock_left(&g_deck) == 120000);
    CHECK(td_netlink_window_open(&g_deck) == TD_NETLINK_CLOSED);
    g_now += 59999;
    td_netlink_deck_tick(&g_deck);
    CHECK(g_failure_saves == 0);
    g_now += 1;
    td_netlink_deck_tick(&g_deck);
    CHECK(g_failure_saves == 1 && g_saved_failures == 5 && g_saved_lock == 60000);
    g_now += 60000;
    td_netlink_deck_tick(&g_deck);
    CHECK(g_saved_lock == 0 && td_netlink_lock_left(&g_deck) == 0);
    CHECK(td_netlink_window_open(&g_deck) == TD_NETLINK_REASON_NONE);
    /* A stored count of 5 means the next failure locks for 30 s * 2^3. */
    connect_from(&g_conn, PEER_A);
    push_code();
    feed_vec(&g_conn, "pairing.wire.request");
    start_attempt(&g_conn);
    feed_vec(&g_conn, "negative.wrong_code.proof");
    CHECK(g_saved_failures == 6 && g_saved_lock == 240000);
}

/* A window whose connection ends before a commit counts once, unless a person ended it, it expired, or a failed proof
 * in it already counted. */
static void test_dropped_window(void)
{
    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_PEER);
    CHECK(g_deck.pair_failures == 1 && g_saved_failures == 1 && !g_deck.window.open);

    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    start_attempt(&g_conn);
    feed_vec(&g_conn, "negative.wrong_code.proof");
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_PEER);
    CHECK(g_deck.pair_failures == 1);

    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_RADIO);
    CHECK(g_deck.pair_failures == 1);

    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    td_netlink_window_cancel(&g_deck);
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_CANCELLED));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_deck.pair_failures == 0 && g_failure_saves == 0);

    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    g_now += TD_NETLINK_WINDOW_MS;
    td_netlink_deck_tick(&g_deck);
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_EXPIRED));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_deck.pair_failures == 0 && td_netlink_backoff_left(&g_deck, PEER_A, 4) == 0);

    /* A missed attempt deadline counts toward both. */
    fresh_deck(false, 0, 0);
    bind_window(&g_conn, PEER_A);
    start_attempt(&g_conn);
    g_now += TD_NETLINK_ATTEMPT_MS;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_EXPIRED));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_deck.pair_failures == 1 && td_netlink_backoff_left(&g_deck, PEER_A, 4) == 1000);
}

/* Only the Device opens windows; pair.request finds `closed` without one and `full` with four Pairings. */
static void test_pair_request_refusals(void)
{
    fresh_deck(false, 0, 0);
    connect_from(&g_conn, PEER_A);
    CHECK(feed_vec(&g_conn, "pairing.wire.request") == TD_NETLINK_SEND_CLOSE && out_is("errors.closed"));
    CHECK(!g_deck.window.open);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_A, 4) == 0);

    fresh_deck(true, 0, 0);
    CHECK(td_netlink_window_open(&g_deck) == TD_NETLINK_REASON_NONE);
    connect_from(&g_conn, PEER_A);
    CHECK(feed_vec(&g_conn, "pairing.wire.request") == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_FULL));
    CHECK(g_deck.window.open && !g_deck.window.bound);

    fresh_deck(false, 0, 0);
    g_deck.pairings.count = TD_NETLINK_PAIRINGS_MAX;
    CHECK(td_netlink_window_open(&g_deck) == TD_NETLINK_FULL);

    /* An item of version 1 gets `version`. */
    fresh_deck(false, 0, 0);
    td_netlink_window_open(&g_deck);
    connect_from(&g_conn, PEER_A);
    VEC(request, "pairing.wire.request");
    request[3] = 1;
    CHECK(feed(&g_conn, request, request_len) == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_BAD_VERSION));
    CHECK(out_is("errors.version") && out_is("negative.version_error.error"));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_A, 4) == 1000);

    /* The version error item itself with item version 1 is malformed: the Device answers `version` and closes. */
    fresh_deck(false, 0, 0);
    td_netlink_window_open(&g_deck);
    connect_from(&g_conn, PEER_A);
    CHECK(feed_vec(&g_conn, "negative.version_error.v1_item.wire") == TD_NETLINK_SEND_CLOSE
          && out_is("errors.version"));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
}

/* Every accepted fixture name binds a window, and every refused one closes the carrier connection. */
static void test_companion_names(void)
{
    const cJSON *entry;
    int accepted = 0, rejected = 0;
    cJSON_ArrayForEach(entry, at(g_vectors, "companion_names.accepted")) {
        fresh_deck(false, 0, 0);
        td_netlink_window_open(&g_deck);
        connect_from(&g_conn, PEER_A);
        uint8_t item[128];
        const size_t len = vec_of(entry, "request", item, sizeof item);
        CHECK(feed(&g_conn, item, len) == TD_NETLINK_SEND && g_out.item[2] == TD_NETLINK_PAIR_OFFER);
        accepted++;
    }
    cJSON_ArrayForEach(entry, at(g_vectors, "companion_names.rejected")) {
        fresh_deck(false, 0, 0);
        td_netlink_window_open(&g_deck);
        connect_from(&g_conn, PEER_A);
        uint8_t item[128];
        const size_t len = vec_of(entry, "request", item, sizeof item);
        const td_netlink_action_t action = feed(&g_conn, item, len);
        CHECK(!strcmp(at(entry, "expected")->valuestring, "close"));
        CHECK(action == TD_NETLINK_SEND_CLOSE || action == TD_NETLINK_CLOSE);
        if (action != TD_NETLINK_SEND_CLOSE && action != TD_NETLINK_CLOSE) {
            fprintf(stderr, "accepted a refused name: %s\n", at(entry, "label")->valuestring);
        }
        CHECK(!g_deck.window.bound);
        rejected++;
    }
    CHECK(accepted == 5 && rejected == 30);
    /* The pinned format ranges are the ones the vectors carry. */
    CHECK(cJSON_GetArraySize(at(g_vectors, "companion_names.format_ranges")) == 21);
}

/* ---- Sessions and records ---- */

static void test_session_and_record_vectors(void)
{
    fresh_deck(true, 0, 0);
    open_session(&g_conn, PEER_A);
    CHECK(td_netlink_conn_proven(&g_conn));
    const cJSON *record;
    int count = 0;
    cJSON_ArrayForEach(record, at(g_vectors, "records")) {
        uint8_t wire[1100], plain[1100], body[1100];
        const size_t wire_len = vec_of(record, "wire", wire, sizeof wire);
        const size_t plain_len = vec_of(record, "plaintext", plain, sizeof plain);
        if (!strcmp(at(record, "direction")->valuestring, "c2d")) {
            CHECK(td_netlink_conn_record(&g_conn, &g_deck, wire + 2, wire_len - 2, body));
            CHECK(!memcmp(body, plain, plain_len));
        } else {
            CHECK(td_netlink_conn_send(&g_conn, plain, plain_len, body));
            CHECK(!memcmp(body, wire, wire_len));
        }
        const cJSON *fragments = cJSON_GetObjectItemCaseSensitive(record, "ble_fragments");
        for (int mtu = 0; fragments && mtu < 2; mtu++) {
            const cJSON *list = at(fragments, mtu ? "att_mtu_247" : "att_mtu_23");
            uint8_t buf[1100];
            td_netlink_reasm_t r;
            td_netlink_reasm_init(&r, mtu ? 244 : 20, buf, sizeof buf);
            CHECK(td_netlink_reasm_records(&r));
            const cJSON *part;
            const uint8_t *item = NULL;
            size_t item_len = 0;
            td_netlink_frag_t last = TD_NETLINK_FRAG_CLOSE;
            cJSON_ArrayForEach(part, list) {
                uint8_t chunk[300];
                const size_t n = unhex(part->valuestring, chunk, sizeof chunk);
                last = td_netlink_reasm_push(&r, chunk, n, &item, &item_len);
            }
            CHECK(last == TD_NETLINK_FRAG_ITEM && item_len == wire_len && !memcmp(item, wire, wire_len));
        }
        count++;
    }
    CHECK(count == 4);

    /* The tampered record closes the session. */
    fresh_deck(true, 0, 0);
    open_session(&g_conn, PEER_A);
    VEC(tampered, "tampered_record.wire");
    uint8_t body[1100];
    CHECK(!strcmp(at(g_vectors, "tampered_record.expected")->valuestring, "close"));
    CHECK(!td_netlink_conn_record(&g_conn, &g_deck, tampered + 2, tampered_len - 2, body));
    CHECK(!td_netlink_conn_record(&g_conn, &g_deck, tampered + 2, tampered_len - 2, body));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_A, 4) == 0);
}

/* A share that the X25519 rule refuses closes the handshake with no error item and counts as malformed. */
static void expect_share_closes(const uint8_t share[32], const char *label)
{
    fresh_deck(true, 0, 0);
    connect_from(&g_conn, PEER_A);
    uint8_t open[58];
    vec("session.wire.open", open, sizeof open);
    memcpy(open + 2 + 2 + 6 + 16, share, 32);
    const td_netlink_action_t action = feed(&g_conn, open, sizeof open);
    CHECK(action == TD_NETLINK_CLOSE && g_out.len == 0);
    if (action != TD_NETLINK_CLOSE) {
        fprintf(stderr, "share accepted: %s\n", label);
    }
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_conn.end == TD_NETLINK_END_MALFORMED && td_netlink_backoff_left(&g_deck, PEER_A, 4) == 1000);
}

static void test_x25519_negatives(void)
{
    const cJSON *entry;
    int count = 0;
    cJSON_ArrayForEach(entry, at(g_vectors, "negative.x25519.small_order")) {
        uint8_t share[32];
        CHECK(vec_of(entry, "share", share, sizeof share) == 32);
        CHECK(!strcmp(at(entry, "expected")->valuestring, "close"));
        expect_share_closes(share, at(entry, "note")->valuestring);
        count++;
    }
    CHECK(count == 12);
    uint8_t share[32];
    vec("negative.x25519.bit_255_set.share", share, sizeof share);
    expect_share_closes(share, "bit 255 set");
    vec("non_contributory_share.device_share", share, sizeof share);
    expect_share_closes(share, "non-contributory");
    /* An honest client share with bit 255 set closes too, though masking would agree. */
    vec("session.client_share", share, sizeof share);
    share[31] |= 0x80;
    expect_share_closes(share, "honest share, bit 255 set");
}

static void test_session_negatives(void)
{
    static const char *const auth[] = { "negative.wrong_device_id", "negative.unknown_companion_id" };
    for (size_t i = 0; i < 2; i++) {
        char path[96];
        fresh_deck(true, 0, 0);
        connect_from(&g_conn, PEER_A);
        snprintf(path, sizeof path, "%s.open", auth[i]);
        CHECK(feed_vec(&g_conn, path) == TD_NETLINK_SEND_CLOSE);
        snprintf(path, sizeof path, "%s.error", auth[i]);
        CHECK(out_is(path));
        td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
        CHECK(g_rng_fill == 0);
    }

    fresh_deck(true, 0, 0);
    connect_from(&g_conn, PEER_A);
    rng_push_vec("inputs.deviceEphemeral");
    CHECK(feed_vec(&g_conn, "negative.swapped_client_share.open") == TD_NETLINK_SEND);
    CHECK(out_is("negative.swapped_client_share.challenge"));
    CHECK(feed_vec(&g_conn, "negative.swapped_client_share.proof") == TD_NETLINK_CLOSE && g_out.len == 0);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_conn.end == TD_NETLINK_END_PROOF && td_netlink_backoff_left(&g_deck, PEER_A, 4) == 1000);

    /* The Companion's side of a swapped device share: the device_proof no longer checks. */
    VEC(challenge, "negative.swapped_device_share.challenge");
    VEC(client_secret, "inputs.clientEphemeral");
    VEC(client_share, "session.client_share");
    VEC(ltk, "inputs.ltk");
    VEC(device_id, "inputs.deviceId");
    VEC(companion_id, "inputs.companionId");
    uint8_t z[32], t[32], prk[32], deck_key[32], proof[32];
    const uint8_t *device_share = challenge + 4;
    CHECK(td_netlink_x25519(client_secret, device_share, z));
    static const char label[] = "touchdeck/net-v2/session";
    const td_netlink_part_t transcript[] = {
        { label, sizeof label }, { device_id, 6 }, { companion_id, 16 }, { client_share, 32 }, { device_share, 32 },
    };
    const td_netlink_part_t ikm[] = { { ltk, 32 }, { z, 32 } };
    const td_netlink_part_t tp[] = { { t, 32 } };
    CHECK(td_netlink_sha256(transcript, 5, t) && td_netlink_hmac(t, 32, ikm, 2, prk)
          && td_netlink_derive(prk, "session/deck", deck_key) && td_netlink_hmac(deck_key, 32, tp, 1, proof));
    CHECK(!td_netlink_equal(proof, challenge + 4 + 32, 32));

    /* The Companion's side of a tampered wrapped_key: the envelope never opens. */
    VEC(wrap, "pairing.wrap");
    VEC(transcript_hash, "pairing.transcript");
    VEC(client_proof, "pairing.clientProof");
    VEC(deck_proof, "pairing.deckProof");
    uint8_t aad[96], key[48];
    memcpy(aad, transcript_hash, 32);
    memcpy(aad + 32, client_proof, 32);
    memcpy(aad + 64, deck_proof, 32);
    const uint8_t zero[12] = { 0 };
    VEC(accept, "pairing.wire.accept");
    CHECK(td_netlink_open(wrap, zero, aad, sizeof aad, accept + 4 + 32, 48, key) && !memcmp(key, ltk, 32));
    const cJSON *entry;
    int tampered = 0;
    cJSON_ArrayForEach(entry, at(g_vectors, "negative.tampered_wrapped_key")) {
        uint8_t item[100];
        vec_of(entry, "accept", item, sizeof item);
        CHECK(!td_netlink_open(wrap, zero, aad, sizeof aad, item + 4 + 32, 48, key));
        tampered++;
    }
    CHECK(tampered == 2);
}

static void test_record_limits(void)
{
    VEC(c2d, "session.c2d");
    VEC(plain, "negative.counter_limit.plaintext");
    VEC(last, "negative.counter_limit.last");
    VEC(beyond, "negative.counter_limit.beyond");
    const cJSON *limit = at(g_vectors, "negative.counter_limit.limit");
    CHECK(cJSON_IsNumber(limit) && (uint64_t)limit->valuedouble == TD_NETLINK_COUNTER_LIMIT);
    CHECK((uint64_t)at(g_vectors, "negative.counter_limit.last_counter")->valuedouble == TD_NETLINK_COUNTER_LIMIT - 1);
    CHECK((uint64_t)at(g_vectors, "negative.counter_limit.beyond_counter")->valuedouble == TD_NETLINK_COUNTER_LIMIT);
    uint8_t wire[1100], body[1100];
    td_netlink_dir_t sender, receiver;
    td_netlink_dir_init(&sender, TD_NETLINK_C2D, c2d);
    sender.counter = TD_NETLINK_COUNTER_LIMIT - 1;
    CHECK(td_netlink_record_seal(&sender, plain, plain_len, wire) && !memcmp(wire, last, last_len));
    CHECK(!td_netlink_record_seal(&sender, plain, plain_len, wire));
    td_netlink_dir_init(&receiver, TD_NETLINK_C2D, c2d);
    receiver.counter = TD_NETLINK_COUNTER_LIMIT - 1;
    CHECK(td_netlink_record_open(&receiver, last + 2, last_len - 2, body) && !memcmp(body, plain, plain_len));
    CHECK(!td_netlink_record_open(&receiver, beyond + 2, beyond_len - 2, body));
    td_netlink_dir_init(&receiver, TD_NETLINK_C2D, c2d);
    receiver.counter = TD_NETLINK_COUNTER_LIMIT;
    CHECK(!td_netlink_record_open(&receiver, beyond + 2, beyond_len - 2, body));

    /* The largest record opens on a session. */
    VEC(max, "negative.max_record.wire");
    VEC(max_plain, "negative.max_record.plaintext");
    CHECK(max_len == TD_NETLINK_ITEM_MAX);
    fresh_deck(true, 0, 0);
    open_session(&g_conn, PEER_A);
    uint16_t min, cap;
    td_netlink_conn_limits(&g_conn, &min, &cap);
    CHECK(min == 24 && cap == 1048);
    CHECK(td_netlink_conn_record(&g_conn, &g_deck, max + 2, max_len - 2, body) && !memcmp(body, max_plain, max_plain_len));
    /* Replaying it closes the session: only the next counter is accepted. */
    CHECK(!td_netlink_conn_record(&g_conn, &g_deck, max + 2, max_len - 2, body));

    /* A session wears out once fewer than 1024 records remain in either direction, and then ends between requests. */
    fresh_deck(true, 0, 0);
    open_session(&g_conn, PEER_A);
    CHECK(!td_netlink_conn_worn(&g_conn));
    g_conn.tx.counter = TD_NETLINK_COUNTER_LIMIT - TD_NETLINK_COUNTER_MARGIN;
    CHECK(!td_netlink_conn_worn(&g_conn));
    g_conn.tx.counter++;
    CHECK(td_netlink_conn_worn(&g_conn));
    g_conn.tx.counter = 0;
    g_conn.rx.counter = TD_NETLINK_COUNTER_LIMIT - TD_NETLINK_COUNTER_MARGIN + 1;
    CHECK(td_netlink_conn_worn(&g_conn));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);

    /* Length prefixes one past each phase's bounds close before any body is read. */
    const cJSON *entry;
    int count = 0;
    cJSON_ArrayForEach(entry, at(g_vectors, "negative.oversize_items")) {
        uint8_t prefix[2];
        vec_of(entry, "prefix", prefix, sizeof prefix);
        const bool records = !strcmp(at(entry, "phase")->valuestring, "record");
        if (records) {
            fresh_deck(true, 0, 0);
            open_session(&g_conn, PEER_A);
        } else {
            fresh_deck(false, 0, 0);
            connect_from(&g_conn, PEER_A);
        }
        const uint16_t len = (uint16_t)(prefix[0] << 8 | prefix[1]);
        td_netlink_conn_limits(&g_conn, &min, &cap);
        CHECK(len < min || len > cap);
        const td_netlink_action_t action = td_netlink_conn_bad_length(&g_conn, &g_deck, &g_out);
        CHECK(action == (records ? TD_NETLINK_CLOSE : TD_NETLINK_SEND_CLOSE));
        uint8_t buf[1100];
        td_netlink_reasm_t r;
        td_netlink_reasm_init(&r, TD_NETLINK_FRAGMENT_UNKNOWN_MAX, buf, sizeof buf);
        if (records) {
            td_netlink_reasm_records(&r);
        }
        const uint8_t *item;
        size_t item_len;
        CHECK(td_netlink_reasm_push(&r, prefix, 2, &item, &item_len) == TD_NETLINK_FRAG_CLOSE);
        count++;
    }
    CHECK(count == 3);
}

static int run_credit(const cJSON *v, int *accepted)
{
    td_netlink_credit_t c;
    td_netlink_credit_init(&c, (uint32_t)at(v, "initial_credit")->valueint);
    *accepted = 0;
    const cJSON *event;
    cJSON_ArrayForEach(event, at(v, "events")) {
        const cJSON *enable = cJSON_GetObjectItemCaseSensitive(event, "enable");
        const cJSON *write = cJSON_GetObjectItemCaseSensitive(event, "write");
        const cJSON *repeat = cJSON_GetObjectItemCaseSensitive(event, "repeat");
        uint8_t value[8];
        if (enable) {
            td_netlink_credit_enable(&c, !strcmp(enable->valuestring, "tx") ? TD_NETLINK_NOTIFY_TX
                                                                             : TD_NETLINK_NOTIFY_FLOW, value);
        }
        for (int i = 0; write && i < (repeat ? repeat->valueint : 1); i++) {
            if (!td_netlink_credit_write(&c, (size_t)write->valueint)) {
                return 0;
            }
            *accepted += write->valueint;
        }
    }
    return 1;
}

static void test_ble_credit(void)
{
    int accepted;
    CHECK(run_credit(at(g_vectors, "negative.write_before_credit"), &accepted) == 0);
    CHECK(run_credit(at(g_vectors, "negative.credit_overrun"), &accepted) == 0);
    CHECK(accepted == at(g_vectors, "negative.credit_overrun.accepted_bytes")->valueint);
    td_netlink_credit_t c;
    td_netlink_credit_init(&c, (uint32_t)at(g_vectors, "ble.initial_credit")->valueint);
    uint8_t value[8], want[8];
    CHECK(!td_netlink_credit_enable(&c, TD_NETLINK_NOTIFY_FLOW, value));
    CHECK(td_netlink_credit_enable(&c, TD_NETLINK_NOTIFY_TX, value));
    vec("ble.initial_credit_value", want, sizeof want);
    CHECK(!memcmp(value, want, 8));
    CHECK(td_netlink_credit_write(&c, 2100));
    CHECK(td_netlink_credit_grant(&c, 2120, value) && td_netlink_credit_write(&c, 20));
    CHECK(!td_netlink_credit_grant(&c, 2100, value));
}

/* ---- Ownership, backoff and USB ---- */

static void test_busy_items(void)
{
    VEC(busy, "errors.busy");
    CHECK(busy_len == sizeof td_netlink_busy_item && !memcmp(busy, td_netlink_busy_item, busy_len));
}

/* Per-peer backoff counts only failures the peer caused, doubles to 60 s, lets a stored companion_id through, and
 * resets at session.ready. */
static void test_backoff(void)
{
    fresh_deck(true, 0, 0);
    connect_from(&g_conn, PEER_A);
    CHECK(feed_vec(&g_conn, "negative.unknown_companion_id.open") == TD_NETLINK_SEND_CLOSE);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_A, 4) == 1000);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_B, 4) == 0);

    /* Anything but a session.open naming a stored companion_id gets busy, and that busy doesn't count. */
    td_netlink_window_open(&g_deck);
    connect_from(&g_conn, PEER_A);
    CHECK(feed_vec(&g_conn, "pairing.wire.request") == TD_NETLINK_SEND_CLOSE && out_is("errors.busy"));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_A, 4) == 1000);
    VEC(open, "session.wire.open");
    CHECK(td_netlink_names_stored(&g_deck, open + 2, open_len - 2));
    VEC(unknown, "negative.unknown_companion_id.open");
    CHECK(!td_netlink_names_stored(&g_deck, unknown + 2, unknown_len - 2));
    open_session(&g_conn, PEER_A);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_PEER);
    CHECK(g_conn.end == TD_NETLINK_END_READY && td_netlink_backoff_left(&g_deck, PEER_A, 4) == 0);

    /* Doubling to the cap. */
    uint32_t want = 1000;
    for (int i = 0; i < 8; i++) {
        connect_from(&g_conn, PEER_B);
        td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_PEER);
        CHECK(g_conn.end == TD_NETLINK_END_PEER_CLOSED);
        CHECK(td_netlink_backoff_left(&g_deck, PEER_B, 4) == want);
        want = want * 2 > TD_NETLINK_BACKOFF_CAP_MS ? TD_NETLINK_BACKOFF_CAP_MS : want * 2;
    }
    g_now += TD_NETLINK_BACKOFF_CAP_MS;
    CHECK(td_netlink_backoff_left(&g_deck, PEER_B, 4) == 0);

    /* None of these count: a takeover, a refusal while USB is online, a radio loss, closed and full. */
    fresh_deck(true, 0, 0);
    connect_from(&g_conn, PEER_A);
    td_netlink_conn_end(&g_conn, &g_deck, TD_NETLINK_END_TAKEOVER);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    g_usb = true;
    connect_from(&g_conn, PEER_A);
    CHECK(feed_vec(&g_conn, "session.wire.open") == TD_NETLINK_SEND_CLOSE && out_is("errors.busy"));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_PEER);
    CHECK(g_conn.end == TD_NETLINK_END_USB_ONLINE);
    g_usb = false;
    connect_from(&g_conn, PEER_A);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_RADIO);
    connect_from(&g_conn, PEER_A);
    feed_vec(&g_conn, "pairing.wire.request");
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);
    CHECK(g_conn.end == TD_NETLINK_END_CLOSED);
    CHECK(td_netlink_backoff_left(&g_deck, PEER_A, 4) == 0);

    /* The first item must arrive within 2 s. */
    connect_from(&g_conn, PEER_A);
    g_now += TD_NETLINK_FIRST_ITEM_MS - 1;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_NOTHING);
    g_now += 1;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_EXPIRED));
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_PEER);
    CHECK(g_conn.end == TD_NETLINK_END_DEADLINE && td_netlink_backoff_left(&g_deck, PEER_A, 4) == 1000);

    /* At least eight peers are kept; a ninth evicts the oldest. */
    fresh_deck(false, 0, 0);
    for (uint8_t i = 0; i < 9; i++) {
        const uint8_t peer[4] = { 198, 51, 100, i };
        connect_from(&g_conn, peer);
        td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_PEER);
        g_now += 10;
    }
    const uint8_t first[4] = { 198, 51, 100, 0 }, second[4] = { 198, 51, 100, 1 };
    CHECK(td_netlink_backoff_left(&g_deck, first, 4) == 0 && td_netlink_backoff_left(&g_deck, second, 4) > 0);
}

/* USB wins: session.open gets busy while USB is online, and a session closes when USB comes online. Pairing works. */
static void test_usb_wins(void)
{
    fresh_deck(true, 0, 0);
    open_session(&g_conn, PEER_A);
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_NOTHING);
    g_usb = true;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_CLOSE && g_out.len == 0);
    td_netlink_conn_close(&g_conn, &g_deck, TD_NETLINK_GONE_DEVICE);

    fresh_deck(false, 0, 0);
    g_usb = true;
    bind_window(&g_conn, PEER_A);
    CHECK(out_is("pairing.wire.offer"));
}

/* Liveness: 5 s to the first hello, then 5 s from the last valid record. Session setup has 10 s. */
static void test_session_liveness(void)
{
    fresh_deck(true, 0, 0);
    open_session(&g_conn, PEER_A);
    g_now += TD_NETLINK_HELLO_MS;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_CLOSE);

    fresh_deck(true, 0, 0);
    open_session(&g_conn, PEER_A);
    td_netlink_conn_hello(&g_conn);
    g_now += 4000;
    uint8_t wire[1100], body[1100];
    const cJSON *first = cJSON_GetArrayItem(at(g_vectors, "records"), 0);
    const size_t len = vec_of(first, "wire", wire, sizeof wire);
    CHECK(td_netlink_conn_record(&g_conn, &g_deck, wire + 2, len - 2, body));
    g_now += 4000;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_NOTHING);
    g_now += 1000;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_CLOSE);

    fresh_deck(true, 0, 0);
    connect_from(&g_conn, PEER_A);
    rng_push_vec("inputs.deviceEphemeral");
    feed_vec(&g_conn, "session.wire.open");
    g_now += TD_NETLINK_SETUP_MS;
    CHECK(td_netlink_conn_tick(&g_conn, &g_deck, &g_out) == TD_NETLINK_SEND_CLOSE && out_is_error(TD_NETLINK_EXPIRED));
}

static void test_forget(void)
{
    fresh_deck(true, 2, 0);
    td_netlink_window_open(&g_deck);
    CHECK(td_netlink_forget(&g_deck));
    CHECK(g_deck.pairings.count == 0 && g_saved.count == 0 && g_pairing_saves == 1);
    CHECK(g_deck.pair_failures == 0 && g_saved_failures == 0 && !g_deck.window.open);
    fresh_deck(true, 4, 500000);
    CHECK(td_netlink_forget(&g_deck) && td_netlink_lock_left(&g_deck) == 0 && g_saved_lock == 0);
    g_pairings_fail = true;
    CHECK(!td_netlink_forget(&g_deck) && g_deck.pairings.count == 0);
}

int main(int argc, char **argv)
{
    if (argc != 2) {
        fprintf(stderr, "usage: netlink_test <net-v2-vectors.json>\n");
        return 2;
    }
    FILE *f = fopen(argv[1], "rb");
    if (!f) {
        perror(argv[1]);
        return 2;
    }
    fseek(f, 0, SEEK_END);
    const long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *text = malloc((size_t)size + 1);
    const size_t got = fread(text, 1, (size_t)size, f);
    fclose(f);
    text[got] = '\0';
    g_vectors = cJSON_Parse(text);
    free(text);
    if (!g_vectors || !td_netlink_crypto_init()) {
        fprintf(stderr, "vectors or crypto unavailable\n");
        return 2;
    }
    CHECK(!strcmp(at(g_vectors, "protocol")->valuestring, "touchdeck/net-v2"));

    static const struct {
        void (*fn)(void);
        const char *name;
    } tests[] = {
        { test_primitives_match_vectors, "primitives" },
        { test_pairing_vectors, "pairing vectors" },
        { test_wrong_code_then_right, "wrong code then right" },
        { test_commit_storage_failure, "commit storage failure" },
        { test_lockout, "lockout" },
        { test_lockout_survives_reboot, "lockout across a reboot" },
        { test_dropped_window, "dropped window" },
        { test_pair_request_refusals, "pair.request refusals" },
        { test_companion_names, "companion names" },
        { test_session_and_record_vectors, "session and record vectors" },
        { test_x25519_negatives, "x25519 negatives" },
        { test_session_negatives, "session negatives" },
        { test_record_limits, "record limits" },
        { test_ble_credit, "ble credit" },
        { test_busy_items, "busy items" },
        { test_backoff, "per-peer backoff" },
        { test_usb_wins, "usb wins" },
        { test_session_liveness, "session liveness" },
        { test_forget, "forget" },
    };
    const size_t count = sizeof tests / sizeof tests[0];
    for (size_t i = 0; i < count; i++) {
        const int before = g_failures;
        tests[i].fn();
        printf("%s %s\n", g_failures == before ? "ok  " : "FAIL", tests[i].name);
    }
    cJSON_Delete(g_vectors);
    if (g_failures) {
        fprintf(stderr, "%d of %d checks failed\n", g_failures, g_checks);
        return 1;
    }
    printf("netlink: %u tests passed, %d checks\n", (unsigned)count, g_checks);
    return 0;
}
