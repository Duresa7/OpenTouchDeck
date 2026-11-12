#include "td_netlink_service.h"
#include "td_netlink_priv.h"
#include "td_netlink.h"
#include "td_netlink_manage.h"
#include "td_link.h"
#include "td_link_carrier.h"
#include "td_frame.h"
#include "power.h"
#include "ui.h"
#include <errno.h>
#include <fcntl.h>
#include <stdatomic.h>
#include <string.h>
#include "sdkconfig.h"
#include "esp_attr.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "nvs_flash.h"
#include "freertos/FreeRTOS.h"
#include "freertos/idf_additions.h"
#include "freertos/message_buffer.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "lwip/sockets.h"

static const char *TAG = "netlink";

/* The net task's stack is in PSRAM, with the rest of its state. A flash write asserts on a PSRAM stack, so each NVS
 * save runs on a short-lived task with an internal stack while the net task waits. Crypto runs on the net task, below
 * the UI's priority. */
#define TASK_STACK 8192
#define SAVE_STACK 3584
#define TASK_PRIORITY 2
#define TASK_CORE 1
#define POLL_MS 10
/* With no connection the task only watches the listener and the window, so it wakes less often. */
#define IDLE_POLL_MS 100
#define WRITE_MS 1000
#define REFUSE_MS 100
#define DRAIN_MS 100
#define OUT_RECORDS 4
/* Sockets the Device is closing: each writes its last error item and drains from the poll loop, so a refusal never
 * blocks the net task. */
#define CLOSERS 4
#define CLOSER_ITEM_MAX 8
/* An outgoing record in s_out: the generation of the carrier connection it belongs to, then the frame body. */
#define QUEUED_HEADER 4
#define SEND_WAIT_MS 100
#define COMMAND_WAIT_MS 3000
#define SCREEN_MS 1000

typedef enum {
    CMD_NONE,
    /* net.pair, net.list or net.forget from USB or the console, answered into s_cmd_reply. */
    CMD_MANAGE,
} cmd_t;

/* What the Settings panel on the Device asks for. It never waits for the answer. */
typedef enum {
    UI_NONE,
    UI_PAIR,
    UI_FORGET_ONE,
    UI_FORGET_ALL,
} ui_cmd_t;

/* What net.state reports, published by the net task. */
typedef struct {
    bool listening;
    bool session;
    uint8_t companion_id[TD_NETLINK_COMPANION_ID_LEN];
    char name[TD_NETLINK_NAME_MAX + 1];
    uint32_t since_ms;
    uint8_t paired;
    bool pairing;
    uint32_t pair_failures;
    uint32_t locked_ms;
} snapshot_t;

/* One TCP connection's item reader. */
typedef struct {
    int fd;
    uint8_t peer[TD_NETLINK_PEER_LEN];
    uint8_t peer_len;
    uint32_t started_ms;
    uint8_t head[2];
    bool in_body;
    size_t want;
    size_t got;
} reader_t;

/* A socket the Device is closing: it writes its last item within REFUSE_MS and reads whatever still arrives for
 * DRAIN_MS, so the peer reads that item instead of losing it to a reset. */
typedef struct {
    int fd;
    uint8_t item[CLOSER_ITEM_MAX];
    uint8_t len;
    uint8_t sent;
    bool draining;
    uint32_t until_ms;
} closer_t;

/* Everything the net task owns, in PSRAM, so only the save task and the SRP arithmetic touch internal RAM. */
typedef struct {
    td_netlink_deck_t deck;
    td_netlink_conn_t conn;
    reader_t owner;
    /* Connections waiting for their first item while the owner is unproven, each in its deck.waiters slot: a
     * session.open with a stored Pairing's open_tag takes over. */
    reader_t waiters[TD_NETLINK_WAITERS];
    uint8_t waiter_body[TD_NETLINK_WAITERS][TD_NETLINK_HANDSHAKE_MAX];
    closer_t closers[CLOSERS];
    td_netlink_out_t item;
    uint8_t body[TD_NETLINK_SEALED_MAX];
    uint8_t plain[TD_NETLINK_PLAIN_MAX];
    uint8_t queued[QUEUED_HEADER + TD_NETLINK_PLAIN_MAX];
    uint8_t wire[TD_NETLINK_RECORD_WIRE_MAX];
} net_t;

static bool carrier_send(const td_link_carrier_t *carrier, const td_frame_t *frame);

static const td_link_carrier_t s_carrier = { .send = carrier_send, .name = "wifi", .network = true };

static MessageBufferHandle_t s_out;
static SemaphoreHandle_t s_cmd_lock;
static SemaphoreHandle_t s_cmd_done;
static SemaphoreHandle_t s_save_done;
static _Atomic cmd_t s_cmd;
static const cJSON *s_cmd_request;
static cJSON *s_cmd_reply;
static portMUX_TYPE s_lock = portMUX_INITIALIZER_UNLOCKED;
static snapshot_t s_snap;
/* The Pairings as the Settings panel lists them, without keys, and its requests. In PSRAM with the rest of the radio
 * build's .bss, under s_lock. */
static EXT_RAM_BSS_ATTR ui_paired_apps_t s_apps;
static _Atomic ui_cmd_t s_ui_cmd;
static EXT_RAM_BSS_ATTR char s_ui_app_id[TD_NETLINK_APP_ID_LEN + 1];
static atomic_bool s_ready;
static atomic_bool s_cancel;
static atomic_bool s_running;

/* Net task only. */
static net_t *n;
static int s_listen = -1;
static bool s_shown;
static bool s_shown_bound;
static uint32_t s_shown_left;
static uint32_t s_screen_ms;

static uint32_t now_ms(void)
{
    return (uint32_t)(esp_timer_get_time() / 1000);
}

/* ---- The environment the core runs in ---- */

static uint32_t clock_ms(void *ctx)
{
    (void)ctx;
    return now_ms();
}

static void fill_random(void *ctx, uint8_t *out, size_t len)
{
    (void)ctx;
    esp_fill_random(out, len);
}

static bool usb_online(void *ctx)
{
    (void)ctx;
    return td_link_usb_online();
}

typedef struct {
    const td_netlink_pairings_t *pairings;
    uint32_t pair_failures;
    uint32_t lock_left_ms;
    bool ok;
} save_job_t;

static void save_task(void *arg)
{
    save_job_t *job = arg;
    job->ok = job->pairings ? td_netlink_nvs_save_pairings(job->pairings)
                            : td_netlink_nvs_save_failures(job->pair_failures, job->lock_left_ms);
    xSemaphoreGive(s_save_done);
    vTaskDelete(NULL);
}

static bool run_save(save_job_t *job)
{
    if (xTaskCreatePinnedToCore(save_task, "td_netlink_nvs", SAVE_STACK, job, TASK_PRIORITY + 1, NULL, TASK_CORE)
        != pdPASS) {
        return false;
    }
    xSemaphoreTake(s_save_done, portMAX_DELAY);
    return job->ok;
}

static bool save_pairings(void *ctx, const td_netlink_pairings_t *pairings)
{
    (void)ctx;
    save_job_t job = { .pairings = pairings };
    return run_save(&job);
}

static bool save_failures(void *ctx, uint32_t pair_failures, uint32_t lock_left_ms)
{
    (void)ctx;
    save_job_t job = { .pair_failures = pair_failures, .lock_left_ms = lock_left_ms };
    return run_save(&job);
}

static const td_netlink_env_t s_env = {
    .rng = fill_random,
    .clock = clock_ms,
    .save_pairings = save_pairings,
    .save_failures = save_failures,
    .usb_online = usb_online,
};

/* ---- Sockets ---- */

static bool write_all(int fd, const uint8_t *buf, size_t len, uint32_t timeout_ms)
{
    const int64_t until = esp_timer_get_time() + (int64_t)timeout_ms * 1000;
    size_t done = 0;
    while (done < len) {
        const ssize_t sent = send(fd, buf + done, len - done, 0);
        if (sent > 0) {
            done += (size_t)sent;
            continue;
        }
        if (sent < 0 && errno != EAGAIN && errno != EWOULDBLOCK) {
            return false;
        }
        const int64_t left = until - esp_timer_get_time();
        if (left <= 0) {
            return false;
        }
        fd_set w;
        FD_ZERO(&w);
        FD_SET(fd, &w);
        struct timeval tv = { .tv_sec = 0, .tv_usec = left < 100000 ? (long)left : 100000 };
        select(fd + 1, NULL, &w, NULL, &tv);
    }
    return true;
}

/* Hands a socket to a closer, with its last item (none for len 0). With every closer in use, the item gets one try
 * and the socket closes at once. */
static void closer_add(int fd, const uint8_t *item, size_t len)
{
    len = len > CLOSER_ITEM_MAX ? 0 : len;
    for (size_t i = 0; i < CLOSERS; i++) {
        closer_t *c = &n->closers[i];
        if (c->fd < 0) {
            *c = (closer_t){ .fd = fd, .len = (uint8_t)len, .until_ms = now_ms() + REFUSE_MS };
            if (len) {
                memcpy(c->item, item, len);
            }
            return;
        }
    }
    if (len) {
        send(fd, item, len, MSG_DONTWAIT);
    }
    close(fd);
}

/* One step of each closer, never waiting. */
static void closers_step(void)
{
    const uint32_t now = now_ms();
    for (size_t i = 0; i < CLOSERS; i++) {
        closer_t *c = &n->closers[i];
        if (c->fd < 0) {
            continue;
        }
        if (!c->draining) {
            if (c->sent < c->len) {
                const ssize_t sent = send(c->fd, c->item + c->sent, c->len - c->sent, MSG_DONTWAIT);
                if (sent > 0) {
                    c->sent += (uint8_t)sent;
                } else if (sent < 0 && errno != EAGAIN && errno != EWOULDBLOCK) {
                    c->sent = c->len;
                }
            }
            if (c->sent < c->len && (int32_t)(now - c->until_ms) < 0) {
                continue;
            }
            c->draining = true;
            c->until_ms = now + DRAIN_MS;
        }
        uint8_t sink[64];
        ssize_t got;
        while ((got = recv(c->fd, sink, sizeof sink, MSG_DONTWAIT)) > 0) {
        }
        const bool open = got < 0 && (errno == EAGAIN || errno == EWOULDBLOCK);
        if (!open || (int32_t)(now - c->until_ms) >= 0) {
            close(c->fd);
            *c = (closer_t){ .fd = -1 };
        }
    }
}

static bool closers_busy(void)
{
    for (size_t i = 0; i < CLOSERS; i++) {
        if (n->closers[i].fd >= 0) {
            return true;
        }
    }
    return false;
}

static void reader_reset(reader_t *r)
{
    r->in_body = false;
    r->want = 0;
    r->got = 0;
}

/* ---- The owner ---- */

static void session_ended(void)
{
    if (atomic_exchange(&s_ready, false)) {
        /* A new generation: whatever the link still holds or answers for this session never reaches the next. */
        td_link_carrier_offline(&s_carrier);
        size_t len;
        while ((len = xMessageBufferReceive(s_out, n->queued, sizeof n->queued, 0)) > 0) {
        }
        td_netlink_wipe(n->queued, sizeof n->queued);
        ESP_LOGI(TAG, "session closed");
    }
}

/* The owner's connection is gone: how says whether the peer or the Device ended it. An orderly close hands the socket
 * to a closer with its last item, if any. */
static void drop_owner_with(bool orderly, const uint8_t *item, size_t len, td_netlink_gone_t how)
{
    if (n->owner.fd < 0) {
        return;
    }
    if (orderly) {
        closer_add(n->owner.fd, item, len);
    } else {
        close(n->owner.fd);
    }
    n->owner.fd = -1;
    reader_reset(&n->owner);
    td_netlink_conn_close(&n->conn, &n->deck, how);
    td_netlink_wipe(n->body, sizeof n->body);
    session_ended();
}

static void drop_owner(bool orderly, td_netlink_gone_t how)
{
    drop_owner_with(orderly, NULL, 0, how);
}

static void act(td_netlink_action_t action)
{
    switch (action) {
    case TD_NETLINK_NOTHING:
        break;
    case TD_NETLINK_SEND:
        if (!write_all(n->owner.fd, n->item.item, n->item.len, WRITE_MS)) {
            drop_owner(false, TD_NETLINK_GONE_PEER);
        } else if (n->conn.state == TD_NETLINK_CONN_READY && !atomic_load(&s_ready)) {
            /* A new session starts offline on the link, until its own hello. */
            td_link_carrier_offline(&s_carrier);
            atomic_store(&s_ready, true);
            ESP_LOGI(TAG, "session ready");
        }
        break;
    case TD_NETLINK_SEND_CLOSE:
        if (n->item.len <= CLOSER_ITEM_MAX) {
            /* An error item: the closer writes it. */
            drop_owner_with(true, n->item.item, n->item.len, TD_NETLINK_GONE_DEVICE);
        } else {
            write_all(n->owner.fd, n->item.item, n->item.len, WRITE_MS);
            drop_owner(true, TD_NETLINK_GONE_DEVICE);
        }
        break;
    case TD_NETLINK_CLOSE:
        drop_owner(false, TD_NETLINK_GONE_DEVICE);
        break;
    }
    td_netlink_wipe(&n->item, sizeof n->item);
}

static void become_owner(int fd, const uint8_t *peer, uint8_t peer_len)
{
    n->owner.fd = fd;
    memcpy(n->owner.peer, peer, peer_len);
    n->owner.peer_len = peer_len;
    n->owner.started_ms = now_ms();
    reader_reset(&n->owner);
    td_netlink_conn_init(&n->conn, &n->deck, peer, peer_len);
}

/* Reads what the owner's socket holds, one length prefix and one body at a time. A bad length closes before any
 * body is read. */
static void read_owner(void)
{
    reader_t *r = &n->owner;
    while (r->fd >= 0) {
        uint8_t *dst = r->in_body ? n->body + r->got : r->head + r->got;
        const size_t want = (r->in_body ? r->want : sizeof r->head) - r->got;
        const ssize_t got = recv(r->fd, dst, want, 0);
        if (got < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            return;
        }
        if (got <= 0) {
            drop_owner(false, TD_NETLINK_GONE_PEER);
            return;
        }
        if (!r->in_body && r->got == 0) {
            td_netlink_conn_item_started(&n->conn, &n->deck);
        }
        r->got += (size_t)got;
        if (!r->in_body) {
            if (r->got < sizeof r->head) {
                continue;
            }
            const uint16_t len = (uint16_t)(r->head[0] << 8 | r->head[1]);
            uint16_t min, max;
            td_netlink_conn_limits(&n->conn, &min, &max);
            r->got = 0;
            if (len < min || len > max) {
                act(td_netlink_conn_bad_length(&n->conn, &n->deck, &n->item));
                return;
            }
            r->in_body = true;
            r->want = len;
            continue;
        }
        if (r->got < r->want) {
            continue;
        }
        const size_t len = r->want;
        reader_reset(r);
        if (n->conn.state != TD_NETLINK_CONN_READY) {
            act(td_netlink_conn_item(&n->conn, &n->deck, n->body, len, &n->item));
        } else if (!td_netlink_conn_record(&n->conn, &n->deck, n->body, len, n->plain)) {
            drop_owner(false, TD_NETLINK_GONE_DEVICE);
        } else {
            td_link_carrier_receive(n->plain, len - TD_NETLINK_TAG_LEN, (void *)&s_carrier);
            td_netlink_wipe(n->plain, sizeof n->plain);
        }
    }
}

/* ---- Connections waiting to take over ---- */

static const td_netlink_conn_t *owner_conn(void)
{
    return n->owner.fd >= 0 ? &n->conn : NULL;
}

/* Hands the waiter's socket to a closer with its last item, or with none. The core slot is already free. */
static void waiter_close(size_t i, const uint8_t *item, size_t len)
{
    closer_add(n->waiters[i].fd, item, len);
    n->waiters[i].fd = -1;
    reader_reset(&n->waiters[i]);
    td_netlink_wipe(n->waiter_body[i], sizeof n->waiter_body[i]);
}

/* A waiter's whole first item: it takes over, becomes the owner of a Device without one, or is refused. */
static void waiter_item(size_t i, size_t len)
{
    const td_netlink_action_t action =
        td_netlink_waiter_item(&n->deck, owner_conn(), (int)i, n->waiter_body[i], len, &n->item);
    if (action == TD_NETLINK_NOTHING) {
        const reader_t waiter = n->waiters[i];
        n->waiters[i].fd = -1;
        reader_reset(&n->waiters[i]);
        if (n->owner.fd >= 0) {
            ESP_LOGI(TAG, "a stored app took over an unproven connection");
            td_netlink_conn_end(&n->conn, &n->deck, TD_NETLINK_END_TAKEOVER);
            drop_owner(false, TD_NETLINK_GONE_DEVICE);
        }
        become_owner(waiter.fd, waiter.peer, waiter.peer_len);
        td_netlink_conn_item_started(&n->conn, &n->deck);
        act(td_netlink_conn_item(&n->conn, &n->deck, n->waiter_body[i], len, &n->item));
    } else {
        waiter_close(i, n->item.item, action == TD_NETLINK_SEND_CLOSE ? n->item.len : 0);
    }
    td_netlink_wipe(n->waiter_body[i], sizeof n->waiter_body[i]);
    td_netlink_wipe(&n->item, sizeof n->item);
}

static void read_waiter(size_t i)
{
    reader_t *r = &n->waiters[i];
    while (r->fd >= 0) {
        uint8_t *dst = r->in_body ? n->waiter_body[i] + r->got : r->head + r->got;
        const size_t want = (r->in_body ? r->want : sizeof r->head) - r->got;
        const ssize_t got = recv(r->fd, dst, want, 0);
        if (got < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            return;
        }
        if (got <= 0) {
            /* A FIN or RST before its first item counts against the peer. */
            td_netlink_waiter_gone(&n->deck, (int)i);
            close(r->fd);
            r->fd = -1;
            reader_reset(r);
            return;
        }
        if (!r->in_body && r->got == 0) {
            td_netlink_waiter_started(&n->deck, (int)i);
        }
        r->got += (size_t)got;
        if (!r->in_body) {
            if (r->got < sizeof r->head) {
                continue;
            }
            const uint16_t len = (uint16_t)(r->head[0] << 8 | r->head[1]);
            if (len < TD_NETLINK_HANDSHAKE_MIN || len > TD_NETLINK_HANDSHAKE_MAX) {
                /* Not a session.open that could take over: busy, like any other first item. */
                td_netlink_waiter_drop(&n->deck, (int)i);
                waiter_close(i, td_netlink_busy_item, sizeof td_netlink_busy_item);
                return;
            }
            r->got = 0;
            r->in_body = true;
            r->want = len;
            continue;
        }
        if (r->got == r->want) {
            waiter_item(i, r->want);
            return;
        }
    }
}

static void tick_waiters(void)
{
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        if (n->waiters[i].fd >= 0
            && td_netlink_waiter_tick(&n->deck, owner_conn(), (int)i, &n->item) == TD_NETLINK_SEND_CLOSE) {
            waiter_close(i, n->item.item, n->item.len);
            td_netlink_wipe(&n->item, sizeof n->item);
        }
    }
}

static void drop_waiters(void)
{
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        if (n->waiters[i].fd >= 0) {
            td_netlink_waiter_drop(&n->deck, (int)i);
            close(n->waiters[i].fd);
            n->waiters[i].fd = -1;
            reader_reset(&n->waiters[i]);
        }
    }
}

/* The backoff key. The listener is IPv4 only, so the zeroed key below is never used today. If an IPv6 listener is
 * added, every IPv6 peer would share that one backoff entry: key them by address (or /64 prefix) first. */
static void peer_of(const struct sockaddr_storage *addr, uint8_t *peer, uint8_t *len)
{
    if (addr->ss_family == AF_INET) {
        const struct sockaddr_in *in = (const struct sockaddr_in *)addr;
        memcpy(peer, &in->sin_addr.s_addr, 4);
        *len = 4;
    } else {
        memset(peer, 0, TD_NETLINK_PEER_LEN);
        *len = TD_NETLINK_PEER_LEN;
    }
}

static void accept_one(void)
{
    struct sockaddr_storage addr;
    socklen_t addr_len = sizeof addr;
    const int fd = accept(s_listen, (struct sockaddr *)&addr, &addr_len);
    if (fd < 0) {
        return;
    }
    fcntl(fd, F_SETFL, fcntl(fd, F_GETFL, 0) | O_NONBLOCK);
    const int one = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
    uint8_t peer[TD_NETLINK_PEER_LEN];
    uint8_t peer_len;
    peer_of(&addr, peer, &peer_len);
    int slot, evicted;
    const td_netlink_admit_t admit = td_netlink_admit(&n->deck, owner_conn(), peer, peer_len, &slot, &evicted);
    if (evicted >= 0) {
        /* The core reused its slot for this connection: the waiter it pushed out gets busy. */
        waiter_close((size_t)evicted, td_netlink_busy_item, sizeof td_netlink_busy_item);
    }
    switch (admit) {
    case TD_NETLINK_ADMIT_OWNER:
        become_owner(fd, peer, peer_len);
        break;
    case TD_NETLINK_ADMIT_WAIT: {
        reader_t *w = &n->waiters[slot];
        w->fd = fd;
        memcpy(w->peer, peer, peer_len);
        w->peer_len = peer_len;
        w->started_ms = now_ms();
        reader_reset(w);
        break;
    }
    case TD_NETLINK_ADMIT_BUSY:
        closer_add(fd, td_netlink_busy_item, sizeof td_netlink_busy_item);
        break;
    }
}

/* ---- Records out ---- */

/* On the td_link task: the frame body goes to the net task, which seals and writes it, tagged with the generation of
 * the session it belongs to. */
static bool carrier_send(const td_link_carrier_t *carrier, const td_frame_t *frame)
{
    (void)carrier;
    static uint8_t queued[QUEUED_HEADER + TD_FRAME_BODY_MAX];
    if (!s_out || !atomic_load(&s_ready)) {
        return false;
    }
    const uint32_t generation = td_link_send_generation();
    memcpy(queued, &generation, QUEUED_HEADER);
    const size_t len = td_frame_encode_body(frame, queued + QUEUED_HEADER, sizeof queued - QUEUED_HEADER);
    const bool sent =
        len && xMessageBufferSend(s_out, queued, QUEUED_HEADER + len, pdMS_TO_TICKS(SEND_WAIT_MS)) == QUEUED_HEADER + len;
    td_netlink_wipe(queued, QUEUED_HEADER + len);
    return sent;
}

static void write_records(void)
{
    size_t got;
    while ((got = xMessageBufferReceive(s_out, n->queued, sizeof n->queued, 0)) > 0) {
        uint32_t generation;
        memcpy(&generation, n->queued, QUEUED_HEADER);
        /* A record of a session that ended, queued before session_ended's flush or answered after it, is dropped. */
        if (n->owner.fd < 0 || n->conn.state != TD_NETLINK_CONN_READY || got <= QUEUED_HEADER
            || generation != td_link_carrier_generation(&s_carrier)) {
            continue;
        }
        const size_t len = got - QUEUED_HEADER;
        if (!td_netlink_conn_send(&n->conn, n->queued + QUEUED_HEADER, len, n->wire)
            || !write_all(n->owner.fd, n->wire, 2 + len + TD_NETLINK_TAG_LEN, WRITE_MS)) {
            drop_owner(false, TD_NETLINK_GONE_DEVICE);
        }
    }
    td_netlink_wipe(n->queued, sizeof n->queued);
    /* Near the record limit, the session ends between requests: nothing waits to go out. */
    if (n->owner.fd >= 0 && td_netlink_conn_worn(&n->conn) && xMessageBufferIsEmpty(s_out)) {
        ESP_LOGI(TAG, "session at its record limit, closing");
        td_netlink_conn_end(&n->conn, &n->deck, TD_NETLINK_END_LOCAL);
        drop_owner(true, TD_NETLINK_GONE_DEVICE);
    }
}

/* ---- The pairing screen and net.state ---- */

static void on_cancel(void)
{
    atomic_store(&s_cancel, true);
}

static void show_window(void)
{
    const td_netlink_window_t *w = &n->deck.window;
    if (!w->open) {
        if (s_shown) {
            ui_pairing_hide();
            s_shown = false;
            ESP_LOGI(TAG, "pairing window closed");
        }
        return;
    }
    const uint32_t left = (td_netlink_window_left(&n->deck) + 999) / 1000;
    const uint32_t now = now_ms();
    if (s_shown && s_shown_bound == w->bound && s_shown_left == left && now - s_screen_ms < SCREEN_MS) {
        return;
    }
    if (!s_shown || s_shown_bound != w->bound || s_shown_left != left) {
        char name[TD_NETLINK_NAME_MAX + 1];
        memcpy(name, w->name, w->name_len);
        name[w->name_len] = '\0';
        char code[TD_NETLINK_CODE_LEN + 1];
        memcpy(code, w->code, TD_NETLINK_CODE_LEN);
        code[TD_NETLINK_CODE_LEN] = '\0';
        ui_pairing_show(w->bound ? code : NULL, name, left, on_cancel);
        /* The window is for a person at the Device: the screen lights, the idle face goes down, and the idle clock
         * starts over each second, so the screen stays lit until the window ends. */
        power_wake();
#if CONFIG_TD_QEMU_TEST
        if (w->bound && !s_shown_bound) {
            /* The QEMU test image only: its client reads the code here, since QEMU has no screen. */
            ESP_LOGI(TAG, "qemu-test pairing code %s", code);
        }
#endif
        td_netlink_wipe(code, sizeof code);
        if (!s_shown) {
            ESP_LOGI(TAG, "pairing window open");
        }
        s_shown = true;
        s_shown_bound = w->bound;
        s_shown_left = left;
    }
    s_screen_ms = now;
}

/* The panel's copy of the Pairings: ids and names only. Its version moves when the list does. */
static void publish_apps(void)
{
    static EXT_RAM_BSS_ATTR ui_paired_apps_t apps;
    memset(&apps, 0, sizeof apps);
    const td_netlink_pairings_t *p = &n->deck.pairings;
    apps.count = p->count;
    for (uint8_t i = 0; i < p->count && i < UI_PAIRED_APPS_MAX; i++) {
        td_netlink_app_id(p->entries[i].companion_id, apps.apps[i].app_id);
        const size_t len = p->entries[i].name_len > UI_PAIRED_APP_NAME_MAX ? UI_PAIRED_APP_NAME_MAX
                                                                            : p->entries[i].name_len;
        memcpy(apps.apps[i].name, p->entries[i].name, len);
    }
    portENTER_CRITICAL(&s_lock);
    if (apps.count != s_apps.count || memcmp(apps.apps, s_apps.apps, sizeof apps.apps) != 0) {
        memcpy(s_apps.apps, apps.apps, sizeof apps.apps);
        s_apps.count = apps.count;
        s_apps.version++;
    }
    s_apps.pairing = n->deck.window.open;
    s_apps.locked_s = (td_netlink_lock_left(&n->deck) + 999) / 1000;
    portEXIT_CRITICAL(&s_lock);
}

static void publish(void)
{
    snapshot_t snap = {
        .listening = s_listen >= 0,
        .paired = n->deck.pairings.count,
        .pairing = n->deck.window.open,
        .pair_failures = n->deck.pair_failures,
        .locked_ms = td_netlink_lock_left(&n->deck),
    };
    if (atomic_load(&s_ready)) {
        snap.session = true;
        memcpy(snap.companion_id, n->conn.companion_id, sizeof snap.companion_id);
        memcpy(snap.name, n->conn.name, n->conn.name_len);
        snap.since_ms = n->conn.ready_ms;
    }
    portENTER_CRITICAL(&s_lock);
    s_snap = snap;
    portEXIT_CRITICAL(&s_lock);
    publish_apps();
}

/* ---- Removing Pairings and opening windows ---- */

static void add_base_fields(cJSON *reply, const snapshot_t *snap);
static snapshot_t snapshot(void);

static void manage_base_fields(void *ctx, cJSON *reply)
{
    (void)ctx;
    publish();
    const snapshot_t snap = snapshot();
    add_base_fields(reply, &snap);
}

/* Before a forget erases anything. A session that ends this way gets no error item, only a close. */
static void end_sessions(void *ctx, const uint8_t *companion_id)
{
    (void)ctx;
    const bool session = n->conn.state == TD_NETLINK_CONN_SESSION_PROOF || n->conn.state == TD_NETLINK_CONN_READY;
    if (n->owner.fd >= 0
        && (!companion_id || (session && !memcmp(n->conn.companion_id, companion_id, TD_NETLINK_COMPANION_ID_LEN)))) {
        td_netlink_conn_end(&n->conn, &n->deck, TD_NETLINK_END_LOCAL);
        drop_owner(false, TD_NETLINK_GONE_DEVICE);
    }
    if (!companion_id) {
        drop_waiters();
    }
}

static const td_netlink_manage_env_t s_manage_env = {
    .base_fields = manage_base_fields,
    .end_sessions = end_sessions,
};

static void run_command(void)
{
    if (atomic_exchange(&s_cmd, CMD_NONE) != CMD_MANAGE) {
        return;
    }
    td_netlink_manage(&n->deck, s_cmd_request, s_cmd_reply, &s_manage_env);
    const char *m = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(s_cmd_request, "m"));
    const bool ok = cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(s_cmd_reply, "ok"));
    ESP_LOGI(TAG, "%s %s", m ? m : "net", ok ? "done" : "refused");
    show_window();
    publish();
    xSemaphoreGive(s_cmd_done);
}

/* What the Settings panel asked for: the same work as the USB messages. Its outcome shows in the panel's next read. */
static void run_ui_command(void)
{
    const ui_cmd_t cmd = atomic_load(&s_ui_cmd);
    if (cmd == UI_NONE) {
        return;
    }
    const char *problem = NULL;
    if (cmd == UI_PAIR) {
        switch (td_netlink_window_open(&n->deck)) {
        case TD_NETLINK_REASON_NONE: break;
        case TD_NETLINK_FULL: problem = UI_PAIRED_APPS_FULL; break;
        default: problem = "Too many wrong codes. Wait, then pair again."; break;
        }
        ESP_LOGI(TAG, "pair new app from the screen%s", problem ? ": refused" : "");
    } else {
        uint8_t id[TD_NETLINK_COMPANION_ID_LEN];
        bool stored = true;
        if (cmd == UI_FORGET_ALL) {
            end_sessions(NULL, NULL);
            stored = td_netlink_forget(&n->deck);
        } else if (td_netlink_parse_app_id(s_ui_app_id, id)) {
            uint8_t removed;
            end_sessions(NULL, id);
            stored = td_netlink_forget_one(&n->deck, id, &removed);
        }
        problem = stored ? NULL : "This Device couldn't save that. Try again.";
        ESP_LOGI(TAG, "%s from the screen%s", cmd == UI_FORGET_ALL ? "pairings erased" : "pairing erased",
                 stored ? "" : ", but the erase failed");
    }
    portENTER_CRITICAL(&s_lock);
    s_apps.problem = problem;
    s_apps.busy = false;
    portEXIT_CRITICAL(&s_lock);
    atomic_store(&s_ui_cmd, UI_NONE);
    show_window();
    publish();
}

static void ui_read(ui_paired_apps_t *out)
{
    portENTER_CRITICAL(&s_lock);
    *out = s_apps;
    portEXIT_CRITICAL(&s_lock);
}

/* The panel asks one thing at a time: a request while one runs is dropped. */
static void ui_request(ui_cmd_t cmd, const char *app_id)
{
    portENTER_CRITICAL(&s_lock);
    const bool idle = !s_apps.busy;
    if (idle) {
        s_apps.busy = true;
        s_apps.problem = NULL;
        if (app_id) {
            strlcpy(s_ui_app_id, app_id, sizeof s_ui_app_id);
        }
    }
    portEXIT_CRITICAL(&s_lock);
    if (idle) {
        atomic_store(&s_ui_cmd, cmd);
    }
}

static void ui_pair(void)
{
    ui_request(UI_PAIR, NULL);
}

static void ui_forget(const char *app_id)
{
    ui_request(UI_FORGET_ONE, app_id);
}

static void ui_forget_all(void)
{
    ui_request(UI_FORGET_ALL, NULL);
}

static const ui_settings_hooks_t s_ui_hooks = {
    .read = ui_read,
    .pair = ui_pair,
    .forget = ui_forget,
    .forget_all = ui_forget_all,
};

static void net_task(void *arg)
{
    (void)arg;
#if CONFIG_TD_QEMU_TEST
    /* The QEMU test image has no USB console to open a window from, so it opens one at boot. */
    td_netlink_window_open(&n->deck);
#endif
    for (;;) {
        write_records();
        run_command();
        run_ui_command();
        if (atomic_exchange(&s_cancel, false)) {
            td_netlink_window_cancel(&n->deck);
        }
        if (n->owner.fd >= 0 && n->conn.state == TD_NETLINK_CONN_READY && !n->conn.hello_seen
            && td_link_carrier_online(&s_carrier)) {
            td_netlink_conn_hello(&n->conn);
        }
        fd_set r;
        FD_ZERO(&r);
        FD_SET(s_listen, &r);
        int top = s_listen;
        bool waiting = false;
        int fds[1 + TD_NETLINK_WAITERS];
        fds[0] = n->owner.fd;
        for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
            fds[1 + i] = n->waiters[i].fd;
            waiting |= n->waiters[i].fd >= 0;
        }
        for (size_t i = 0; i < sizeof fds / sizeof fds[0]; i++) {
            if (fds[i] >= 0) {
                FD_SET(fds[i], &r);
                top = fds[i] > top ? fds[i] : top;
            }
        }
        const bool busy =
            n->owner.fd >= 0 || waiting || closers_busy() || n->deck.window.open || atomic_load(&s_ready);
        struct timeval tv = { .tv_sec = 0, .tv_usec = (busy ? POLL_MS : IDLE_POLL_MS) * 1000 };
        if (select(top + 1, &r, NULL, NULL, &tv) > 0) {
            if (n->owner.fd >= 0 && FD_ISSET(n->owner.fd, &r)) {
                read_owner();
            }
            for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
                if (n->waiters[i].fd >= 0 && FD_ISSET(n->waiters[i].fd, &r)) {
                    read_waiter(i);
                }
            }
            if (FD_ISSET(s_listen, &r)) {
                accept_one();
            }
        }
        td_netlink_deck_tick(&n->deck);
        if (n->owner.fd >= 0) {
            act(td_netlink_conn_tick(&n->conn, &n->deck, &n->item));
        }
        /* A waiter waits for its first item only while the owner can still be taken over, and 2000 ms at most. */
        tick_waiters();
        closers_step();
        show_window();
        publish();
    }
}

static int open_listener(void)
{
    const int fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (fd < 0) {
        return -1;
    }
    const int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(TD_NETLINK_PORT),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    if (bind(fd, (struct sockaddr *)&addr, sizeof addr) != 0 || listen(fd, 2) != 0) {
        close(fd);
        return -1;
    }
    fcntl(fd, F_SETFL, fcntl(fd, F_GETFL, 0) | O_NONBLOCK);
    return fd;
}

/* The service's state, records and task stack live in PSRAM. Only the QEMU test image runs without PSRAM, since QEMU's
 * PSRAM model crashes at boot with this Firmware's settings. */
#if CONFIG_SPIRAM
#define NET_CAPS (MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT)
#else
#define NET_CAPS (MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT)
#endif

esp_err_t td_netlink_start(void)
{
    if (n) {
        return ESP_ERR_INVALID_STATE;
    }
    if (!td_netlink_crypto_init()) {
        return ESP_FAIL;
    }
    n = heap_caps_calloc(1, sizeof *n, NET_CAPS);
    s_out = xMessageBufferCreateWithCaps(OUT_RECORDS * (QUEUED_HEADER + TD_NETLINK_PLAIN_MAX + sizeof(size_t)),
                                         NET_CAPS);
    s_cmd_lock = xSemaphoreCreateMutex();
    s_cmd_done = xSemaphoreCreateBinary();
    s_save_done = xSemaphoreCreateBinary();
    if (!n || !s_out || !s_cmd_lock || !s_cmd_done || !s_save_done) {
        return ESP_ERR_NO_MEM;
    }
    n->owner.fd = -1;
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        n->waiters[i].fd = -1;
    }
    for (size_t i = 0; i < CLOSERS; i++) {
        n->closers[i].fd = -1;
    }
    /* The radio's start set up NVS; a second init is a no-op. */
    if (nvs_flash_init() != ESP_OK) {
        ESP_LOGW(TAG, "NVS unavailable, pairings will not persist");
    }
    /* The device_id is the id a selected link-v2 hello reports. */
    uint8_t device_id[TD_NETLINK_DEVICE_ID_LEN];
    esp_err_t err = esp_efuse_mac_get_default(device_id);
    if (err != ESP_OK) {
        return err;
    }
    td_netlink_pairings_t stored;
    uint32_t pair_failures, lock_left_ms;
    if (!td_netlink_nvs_load(&stored, &pair_failures, &lock_left_ms)) {
        ESP_LOGW(TAG, "stored pairings unreadable, starting with none");
    }
    td_netlink_deck_init(&n->deck, device_id, &stored, pair_failures, lock_left_ms, &s_env);
    td_netlink_wipe(&stored, sizeof stored);
    err = td_link_carrier_add(&s_carrier);
    if (err != ESP_OK) {
        return err;
    }
    s_listen = open_listener();
    if (s_listen < 0) {
        ESP_LOGE(TAG, "listener on %d failed: errno %d", TD_NETLINK_PORT, errno);
    }
    publish();
    if (s_listen < 0) {
        return ESP_FAIL;
    }
    if (xTaskCreatePinnedToCoreWithCaps(net_task, "td_netlink", TASK_STACK, NULL, TASK_PRIORITY, NULL, TASK_CORE,
                                        NET_CAPS) != pdPASS) {
        close(s_listen);
        s_listen = -1;
        publish();
        return ESP_ERR_NO_MEM;
    }
    atomic_store(&s_running, true);
    ui_settings_set_hooks(&s_ui_hooks);
    ESP_LOGI(TAG, "listening on %d, %u paired", TD_NETLINK_PORT, (unsigned)n->deck.pairings.count);
    return ESP_OK;
}

/* ---- Commands from other tasks ---- */

static snapshot_t snapshot(void)
{
    portENTER_CRITICAL(&s_lock);
    const snapshot_t snap = s_snap;
    portEXIT_CRITICAL(&s_lock);
    return snap;
}

/* listening, port, ble and session: the net.state fields the deck doesn't hold. */
static void add_base_fields(cJSON *reply, const snapshot_t *snap)
{
    cJSON_AddBoolToObject(reply, "listening", snap->listening);
    cJSON_AddNumberToObject(reply, "port", TD_NETLINK_PORT);
    cJSON_AddBoolToObject(reply, "ble", false);
    if (snap->session) {
        char app_id[TD_NETLINK_APP_ID_LEN + 1];
        td_netlink_app_id(snap->companion_id, app_id);
        cJSON *session = cJSON_AddObjectToObject(reply, "session");
        cJSON_AddStringToObject(session, "app_id", app_id);
        cJSON_AddStringToObject(session, "app_name", snap->name);
        cJSON_AddStringToObject(session, "carrier", "wifi");
        cJSON_AddNumberToObject(session, "since_ms", snap->since_ms);
    } else {
        cJSON_AddNullToObject(reply, "session");
    }
}

void td_netlink_state_fields(cJSON *reply)
{
    const snapshot_t snap = snapshot();
    add_base_fields(reply, &snap);
    cJSON_AddNumberToObject(reply, "paired", snap.paired);
    cJSON_AddBoolToObject(reply, "pairing", snap.pairing);
    cJSON_AddNumberToObject(reply, "pair_failures", snap.pair_failures);
    cJSON_AddNumberToObject(reply, "locked_ms", snap.locked_ms);
}

bool td_netlink_manage_request(const cJSON *request, cJSON *reply)
{
    if (!atomic_load(&s_running) || xSemaphoreTake(s_cmd_lock, pdMS_TO_TICKS(COMMAND_WAIT_MS)) != pdTRUE) {
        return false;
    }
    xSemaphoreTake(s_cmd_done, 0);
    s_cmd_request = request;
    s_cmd_reply = reply;
    atomic_store(&s_cmd, CMD_MANAGE);
    bool done = xSemaphoreTake(s_cmd_done, pdMS_TO_TICKS(COMMAND_WAIT_MS)) == pdTRUE;
    if (!done) {
        /* Take the command back. If the net task already has it, it is writing the reply: wait for it to finish. */
        cmd_t expected = CMD_MANAGE;
        if (!atomic_compare_exchange_strong(&s_cmd, &expected, CMD_NONE)) {
            done = xSemaphoreTake(s_cmd_done, portMAX_DELAY) == pdTRUE;
        }
    }
    xSemaphoreGive(s_cmd_lock);
    return done;
}
