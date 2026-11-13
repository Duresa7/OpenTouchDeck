#include "td_netlink_service.h"
#include "td_netlink_priv.h"
#include "td_netlink.h"
#include "td_netlink_tcp.h"
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
#define OUT_RECORDS 4
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

typedef struct {
    td_netlink_tcp_t tcp;
    uint8_t queued[QUEUED_HEADER + TD_NETLINK_PLAIN_MAX];
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

static int socket_result(int result)
{
    return result < 0 && (errno == EAGAIN || errno == EWOULDBLOCK) ? TD_NETLINK_TCP_AGAIN : result;
}

static int socket_read(void *ctx, int fd, uint8_t *buf, size_t len)
{
    (void)ctx;
    return socket_result(recv(fd, buf, len, MSG_DONTWAIT));
}

static int socket_write(void *ctx, int fd, const uint8_t *buf, size_t len)
{
    (void)ctx;
    return socket_result(send(fd, buf, len, MSG_DONTWAIT));
}

static void socket_close(void *ctx, int fd)
{
    (void)ctx;
    close(fd);
}

static void socket_wait_write(void *ctx, int fd, uint32_t timeout_ms)
{
    (void)ctx;
    fd_set w;
    FD_ZERO(&w);
    FD_SET(fd, &w);
    struct timeval tv = { .tv_sec = 0, .tv_usec = timeout_ms * 1000 };
    select(fd + 1, NULL, &w, NULL, &tv);
}

static void session_changed(void *ctx, bool ready)
{
    (void)ctx;
    if (!ready) {
        atomic_store(&s_ready, false);
    }
    td_link_carrier_offline(&s_carrier);
    if (ready) {
        atomic_store(&s_ready, true);
    }
    if (!ready) {
        while (xMessageBufferReceive(s_out, n->queued, sizeof n->queued, 0) > 0) {
        }
        td_netlink_wipe(n->queued, sizeof n->queued);
    }
    ESP_LOGI(TAG, "session %s", ready ? "ready" : "closed");
}

static void receive_record(void *ctx, const uint8_t *body, size_t len)
{
    (void)ctx;
    td_link_carrier_receive(body, len, (void *)&s_carrier);
}

static const td_netlink_tcp_io_t s_tcp_io = {
    .read = socket_read, .write = socket_write, .close = socket_close, .wait_write = socket_wait_write,
    .session = session_changed, .receive = receive_record,
};

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
    td_netlink_tcp_accept(&n->tcp, fd, peer, peer_len);
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
        /* A record of a session that ended, queued before session_changed's flush or answered after it, is dropped. */
        if (n->tcp.owner.fd < 0 || n->tcp.conn.state != TD_NETLINK_CONN_READY || got <= QUEUED_HEADER
            || generation != td_link_carrier_generation(&s_carrier)) {
            continue;
        }
        td_netlink_tcp_send(&n->tcp, n->queued + QUEUED_HEADER, got - QUEUED_HEADER);
    }
    td_netlink_wipe(n->queued, sizeof n->queued);
    if (xMessageBufferIsEmpty(s_out)) {
        td_netlink_tcp_flushed(&n->tcp);
    }
}

/* ---- The pairing screen and net.state ---- */

static void on_cancel(void)
{
    atomic_store(&s_cancel, true);
}

static void show_window(void)
{
    const td_netlink_window_t *w = &n->tcp.deck.window;
    if (!w->open) {
        if (s_shown) {
            ui_pairing_hide();
            s_shown = false;
            ESP_LOGI(TAG, "pairing window closed");
        }
        return;
    }
    const uint32_t left = (td_netlink_window_left(&n->tcp.deck) + 999) / 1000;
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
    const td_netlink_pairings_t *p = &n->tcp.deck.pairings;
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
    s_apps.pairing = n->tcp.deck.window.open;
    s_apps.locked_s = (td_netlink_lock_left(&n->tcp.deck) + 999) / 1000;
    portEXIT_CRITICAL(&s_lock);
}

static void publish(void)
{
    snapshot_t snap = {
        .listening = s_listen >= 0,
        .paired = n->tcp.deck.pairings.count,
        .pairing = n->tcp.deck.window.open,
        .pair_failures = n->tcp.deck.pair_failures,
        .locked_ms = td_netlink_lock_left(&n->tcp.deck),
    };
    if (atomic_load(&s_ready)) {
        snap.session = true;
        memcpy(snap.companion_id, n->tcp.conn.companion_id, sizeof snap.companion_id);
        memcpy(snap.name, n->tcp.conn.name, n->tcp.conn.name_len);
        snap.since_ms = n->tcp.conn.ready_ms;
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
    td_netlink_tcp_end(&n->tcp, companion_id);
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
    td_netlink_manage(&n->tcp.deck, s_cmd_request, s_cmd_reply, &s_manage_env);
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
        switch (td_netlink_window_open(&n->tcp.deck)) {
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
            stored = td_netlink_forget(&n->tcp.deck);
        } else if (td_netlink_parse_app_id(s_ui_app_id, id)) {
            uint8_t removed;
            end_sessions(NULL, id);
            stored = td_netlink_forget_one(&n->tcp.deck, id, &removed);
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
    td_netlink_window_open(&n->tcp.deck);
#endif
    for (;;) {
        write_records();
        run_command();
        run_ui_command();
        if (atomic_exchange(&s_cancel, false)) {
            td_netlink_window_cancel(&n->tcp.deck);
        }
        if (n->tcp.owner.fd >= 0 && n->tcp.conn.state == TD_NETLINK_CONN_READY && !n->tcp.conn.hello_seen
            && td_link_carrier_online(&s_carrier)) {
            td_netlink_conn_hello(&n->tcp.conn);
        }
        fd_set r;
        FD_ZERO(&r);
        FD_SET(s_listen, &r);
        int top = s_listen;
        int fds[1 + TD_NETLINK_WAITERS];
        fds[0] = n->tcp.owner.fd;
        for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
            fds[1 + i] = n->tcp.waiters[i].fd;
        }
        for (size_t i = 0; i < sizeof fds / sizeof fds[0]; i++) {
            if (fds[i] >= 0) {
                FD_SET(fds[i], &r);
                top = fds[i] > top ? fds[i] : top;
            }
        }
        const bool busy =
            td_netlink_tcp_busy(&n->tcp) || n->tcp.deck.window.open || atomic_load(&s_ready);
        struct timeval tv = { .tv_sec = 0, .tv_usec = (busy ? POLL_MS : IDLE_POLL_MS) * 1000 };
        if (select(top + 1, &r, NULL, NULL, &tv) > 0) {
            if (n->tcp.owner.fd >= 0 && FD_ISSET(n->tcp.owner.fd, &r)) {
                td_netlink_tcp_read(&n->tcp, n->tcp.owner.fd);
            }
            for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
                if (n->tcp.waiters[i].fd >= 0 && FD_ISSET(n->tcp.waiters[i].fd, &r)) {
                    td_netlink_tcp_read(&n->tcp, n->tcp.waiters[i].fd);
                }
            }
            if (FD_ISSET(s_listen, &r)) {
                accept_one();
            }
        }
        td_netlink_tcp_tick(&n->tcp);
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
    td_netlink_deck_init(&n->tcp.deck, device_id, &stored, pair_failures, lock_left_ms, &s_env);
    td_netlink_tcp_init(&n->tcp, &s_tcp_io);
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
    ESP_LOGI(TAG, "listening on %d, %u paired", TD_NETLINK_PORT, (unsigned)n->tcp.deck.pairings.count);
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
