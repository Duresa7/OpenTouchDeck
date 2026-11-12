/*
 * The link's carrier seam on the host. The production td_link.c, its message dispatcher td_link_msgs.c, the transfer
 * (td_link_transfer.c and td_transfer.c over td_xfer.c), the frame codec and cJSON run unchanged. Frames arrive
 * through the USB console's frame handler, as the console task delivers them, and through a fake carrier. The test
 * plays the link task's turns and checks which carrier each reply and event leaves on, what `link.state` reports,
 * and what a transfer holds and releases while its connection lasts, replaces it or ends. The heap, the firmware
 * update, the Asset upload and the idle face are counting fakes (see `fx`). td_link.c is included so the test can take
 * those turns one at a time.
 */
#include "td_link.c"

#include <stdio.h>
#include <stdlib.h>
#include "esp_app_desc.h"
#include "esp_mac.h"
#include "esp_ota_ops.h"
#include "td_asset.h"
#include "td_device.h"
#include "td_recovery.h"
#include "td_update.h"
#include "td_xfer.h"

/* test_link_carrier.py builds this test a second time with -DCONFIG_TD_RADIO_BUILD=1, as the radio build sets it. */
#ifndef CONFIG_TD_RADIO_BUILD
#define CONFIG_TD_RADIO_BUILD 0
#endif

/* ---- Host stand-ins: clock, FreeRTOS, console ---- */

static int64_t g_now_ms;

int64_t esp_timer_get_time(void)
{
    return g_now_ms * 1000;
}

struct host_stream {
    uint8_t *buf;
    size_t cap, head, used;
    bool owns_buf;
};

StreamBufferHandle_t xStreamBufferCreate(size_t size, size_t trigger)
{
    (void)trigger;
    struct host_stream *s = calloc(1, sizeof *s);
    s->buf = malloc(size);
    s->cap = size;
    s->owns_buf = true;
    return s;
}

/* As FreeRTOS does it: one byte of the storage tells full from empty, so it holds size - 1. The caller owns both. */
StreamBufferHandle_t xStreamBufferCreateStatic(size_t size, size_t trigger, uint8_t *storage,
                                               StaticStreamBuffer_t *buffer)
{
    (void)trigger;
    (void)buffer;
    struct host_stream *s = calloc(1, sizeof *s);
    s->buf = storage;
    s->cap = size - 1;
    return s;
}

BaseType_t xStreamBufferIsEmpty(StreamBufferHandle_t s)
{
    return s->used == 0;
}

size_t xStreamBufferSpacesAvailable(StreamBufferHandle_t s)
{
    return s->cap - s->used;
}

size_t xStreamBufferSend(StreamBufferHandle_t s, const void *data, size_t len, TickType_t wait)
{
    (void)wait;
    if (len > s->cap - s->used) {
        len = s->cap - s->used;
    }
    for (size_t i = 0; i < len; i++) {
        s->buf[(s->head + s->used + i) % s->cap] = ((const uint8_t *)data)[i];
    }
    s->used += len;
    return len;
}

size_t xStreamBufferReceive(StreamBufferHandle_t s, void *data, size_t len, TickType_t wait)
{
    (void)wait;
    if (len > s->used) {
        len = s->used;
    }
    for (size_t i = 0; i < len; i++) {
        ((uint8_t *)data)[i] = s->buf[(s->head + i) % s->cap];
    }
    s->head = (s->head + len) % s->cap;
    s->used -= len;
    return len;
}

void vStreamBufferDelete(StreamBufferHandle_t s)
{
    if (s->owns_buf) {
        free(s->buf);
    }
    free(s);
}

struct host_queue {
    uint8_t *items;
    size_t item, length, head, used;
};

QueueHandle_t xQueueCreate(UBaseType_t length, UBaseType_t item_size)
{
    struct host_queue *q = calloc(1, sizeof *q);
    q->items = malloc((size_t)length * item_size);
    q->item = item_size;
    q->length = length;
    return q;
}

BaseType_t xQueueSend(QueueHandle_t q, const void *item, TickType_t wait)
{
    (void)wait;
    if (q->used == q->length) {
        return pdFALSE;
    }
    memcpy(q->items + ((q->head + q->used) % q->length) * q->item, item, q->item);
    q->used++;
    return pdTRUE;
}

BaseType_t xQueueReceive(QueueHandle_t q, void *item, TickType_t wait)
{
    (void)wait;
    if (!q->used) {
        return pdFALSE;
    }
    memcpy(item, q->items + q->head * q->item, q->item);
    q->head = (q->head + 1) % q->length;
    q->used--;
    return pdTRUE;
}

BaseType_t xQueueReset(QueueHandle_t q)
{
    q->head = q->used = 0;
    return pdPASS;
}

void vQueueDelete(QueueHandle_t q)
{
    free(q->items);
    free(q);
}

struct host_mutex {
    bool held;
};

SemaphoreHandle_t xSemaphoreCreateMutex(void)
{
    return calloc(1, sizeof(struct host_mutex));
}

BaseType_t xSemaphoreTake(SemaphoreHandle_t m, TickType_t wait)
{
    (void)wait;
    assert(!m->held);
    m->held = true;
    return pdTRUE;
}

BaseType_t xSemaphoreGive(SemaphoreHandle_t m)
{
    assert(m->held);
    m->held = false;
    return pdTRUE;
}

void vSemaphoreDelete(SemaphoreHandle_t m)
{
    free(m);
}

BaseType_t xTaskCreate(TaskFunction_t task, const char *name, uint32_t stack, void *arg, UBaseType_t priority,
                       TaskHandle_t *handle)
{
    (void)task; (void)name; (void)stack; (void)arg; (void)priority; (void)handle;
    return pdPASS;
}

/* What the USB console wrote, and the frame handler the link gave it. */
static uint8_t g_usb_wire[4 * TD_FRAME_WIRE_MAX];
static size_t g_usb_len;
static td_rx_frame_fn g_usb_handler;
static void *g_usb_context;

bool td_console_write(const char *s, size_t len)
{
    assert(g_usb_len + len <= sizeof g_usb_wire);
    memcpy(g_usb_wire + g_usb_len, s, len);
    g_usb_len += len;
    return true;
}

void td_console_set_frame_handler(td_rx_frame_fn handler, void *context)
{
    g_usb_handler = handler;
    g_usb_context = context;
}

/* ---- The rest of the Device, as quiet as a Device with the built-in Layout and nothing pressed ---- */

uint32_t ui_live_generation(void) { return 1; }
void ui_live_select(void) {}
bool ui_ready(void) { return false; }
uint8_t ui_page_shown(uint32_t *layout) { *layout = 0; return 0; }
bool ui_page_id(uint8_t index, char *out, size_t size) { (void)index; (void)out; (void)size; return false; }
bool ui_page_report_due(ui_page_report_t *report, uint32_t session, uint32_t generation, uint8_t page)
{
    (void)report; (void)session; (void)generation; (void)page;
    return false;
}
void ui_set_link_online(bool (*online)(void)) { (void)online; }
void ui_layout_info(ui_layout_info_t *out) { memset(out, 0, sizeof *out); }
const char *ui_layout_slot(void) { return "landscape"; }
void ui_widget_copy_text(char *dst, size_t cap, const char *src) { snprintf(dst, cap, "%s", src); }
const char *ui_widget_update(const char *tile, const char *value, const char *caption, float bar, bool stale,
                             ui_widget_state_t *stored)
{
    (void)tile; (void)value; (void)caption; (void)bar; (void)stale; (void)stored;
    return "unknown tile";
}
const char *ui_dashboard_update(const char *page, const ui_dash_row_t *rows, size_t count)
{
    (void)page; (void)rows; (void)count;
    return "unknown page";
}

size_t key_sink_link_next(char *buf, size_t cap) { (void)buf; (void)cap; return 0; }
void key_sink_link_discard(void) {}

/* ---- What a transfer holds, as counting fakes of the Device's heap, update, Asset upload and idle face ---- */

static struct {
    int allocs, frees;
    bool fail_alloc;
    int update_cancels;
    size_t update_bytes;
    bool update_refuse;
    int asset_aborts;
    size_t asset_bytes;
    const char *asset_refuse;
    bool idle_face;
    uint32_t idle_entry;
} fx;

void *heap_caps_malloc(size_t size, unsigned caps)
{
    (void)caps;
    if (fx.fail_alloc) {
        return NULL;
    }
    fx.allocs++;
    return malloc(size);
}

void heap_caps_free(void *ptr)
{
    if (ptr) {
        fx.frees++;
    }
    free(ptr);
}

void td_update_cancel(void) { fx.update_cancels++; }
bool td_update_write(const uint8_t *data, size_t len)
{
    (void)data;
    if (fx.update_refuse) {
        return false;
    }
    fx.update_bytes += len;
    return true;
}
void td_asset_upload_abort(void) { fx.asset_aborts++; }
td_asset_result_t td_asset_upload_write(const uint8_t *data, size_t len)
{
    (void)data;
    if (fx.asset_refuse) {
        return TD_ASSET_STORAGE;
    }
    fx.asset_bytes += len;
    return TD_ASSET_OK;
}
const char *td_asset_result_reason(td_asset_result_t result)
{
    return result == TD_ASSET_OK ? "ok" : "storage";
}
bool power_idle_face(uint32_t *entry)
{
    *entry = fx.idle_entry;
    return fx.idle_face;
}

#define HANDLER(name) \
    void name(const cJSON *request, cJSON *reply) { (void)request; td_link_fail(reply, "not on the host"); }
HANDLER(td_link_layout_put)
HANDLER(td_link_layout_get)
HANDLER(td_link_icon_put)
HANDLER(td_link_icon_list)
HANDLER(td_link_icon_rm)
HANDLER(td_link_settings_get)
HANDLER(td_link_settings_put)
HANDLER(td_link_store_stat)
HANDLER(td_link_xfer_end)
HANDLER(td_link_update_begin)
HANDLER(td_link_update_end)
HANDLER(td_link_asset_stat)
HANDLER(td_link_asset_list)
HANDLER(td_link_asset_put)
HANDLER(td_link_asset_end)
HANDLER(td_link_asset_delete)
HANDLER(td_link_live_set)
HANDLER(td_link_photo_put)
HANDLER(td_link_photo_end)
HANDLER(td_link_idle_get)
HANDLER(td_link_idle_set)
HANDLER(td_link_clock_set)

static esp_app_desc_t g_app = { .version = "host" };

const esp_app_desc_t *esp_app_get_description(void)
{
    return &g_app;
}

const esp_partition_t *esp_ota_get_running_partition(void)
{
    static const esp_partition_t running = { .label = "ota_0" };
    return &running;
}

esp_err_t esp_efuse_mac_get_default(uint8_t *mac)
{
    memset(mac, 0x5a, 6);
    return ESP_OK;
}

td_asset_result_t td_asset_space(td_asset_space_t *out)
{
    memset(out, 0, sizeof *out);
    return TD_ASSET_OK;
}

static char g_name[TD_DEVICE_NAME_MAX + 1] = "Open TouchDeck";
esp_err_t td_device_name_get(char out[TD_DEVICE_NAME_MAX + 1]) { strcpy(out, g_name); return ESP_OK; }
esp_err_t td_device_name_set(const char *name) { (void)name; return ESP_OK; }
bool td_recovery_mode(void) { return false; }

/* ---- The fake carrier ---- */

typedef struct {
    td_link_carrier_t carrier;
    bool refuse;
    int frames, chunks, acks;
    /* td_link_send_generation for the last frame. */
    uint32_t generation;
    td_frame_t last;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
} fake_carrier_t;

static bool fake_send(const td_link_carrier_t *carrier, const td_frame_t *frame)
{
    fake_carrier_t *fake = (fake_carrier_t *)carrier;
    if (fake->refuse) {
        return false;
    }
    fake->frames++;
    fake->chunks += frame->type == TD_FRAME_CHUNK;
    fake->acks += frame->type == TD_FRAME_ACK;
    fake->generation = td_link_send_generation();
    fake->last = *frame;
    memcpy(fake->payload, frame->payload, frame->len);
    fake->payload[frame->len] = '\0';
    fake->last.payload = (const uint8_t *)fake->payload;
    return true;
}

static fake_carrier_t g_fake;

/* ---- Checks ---- */

static int g_failures;

#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            fprintf(stderr, "%s:%d: %s: failed: %s\n", __FILE__, __LINE__, __func__, #cond); \
            g_failures++;                                                        \
        }                                                                        \
    } while (0)

/* Puts the link back to before td_link_start, with nothing sent or received. */
static void reset(void)
{
    if (s_frames) {
        vStreamBufferDelete(s_frames);
        vQueueDelete(s_presses);
        vSemaphoreDelete(s_rx_mutex);
    }
    free(s_frames_storage);
    s_frames_storage = NULL;
    s_frames_secret = false;
    s_frames = NULL;
    s_presses = NULL;
    s_rx_mutex = NULL;
    memset(&s_state, 0, sizeof s_state);
    memset(s_carriers, 0, sizeof s_carriers);
    s_carrier_count = 0;
    s_current = 0;
    s_current_generation = 0;
    s_send_generation = 0;
    /* Whatever the last test left open goes, and the fakes start from nothing. */
    td_transfer_close(td_link_transfer());
    td_transfer_init(td_link_transfer(), td_link_transfer()->ports);
    memset(&fx, 0, sizeof fx);
    g_usb_len = 0;
    g_usb_handler = NULL;
    g_usb_context = NULL;
    for (size_t t = 0; t < TD_LINK_MESSAGE_TABLES; t++) {
        s_tables[t] = NULL;
        s_table_counts[t] = 0;
    }
    memset(&g_fake, 0, sizeof g_fake);
    g_fake.carrier = (td_link_carrier_t){ .send = fake_send, .name = "fake" };
    g_now_ms = 100000;
}

static void start(bool with_fake)
{
    reset();
    CHECK(td_link_start() == ESP_OK);
    CHECK(g_usb_handler != NULL);
    if (with_fake) {
        CHECK(td_link_carrier_add(&g_fake.carrier) == ESP_OK);
    }
}

/* The frame body of a request message, as a carrier hands it to the link. */
static size_t request_body(const char *json, uint8_t seq, uint8_t *body)
{
    uint8_t wire[TD_FRAME_WIRE_MAX];
    td_frame_t frame = { .type = TD_FRAME_MESSAGE, .seq = seq, .len = (uint16_t)strlen(json),
                         .payload = (const uint8_t *)json };
    size_t len = td_frame_encode(&frame, wire, sizeof wire);
    td_frame_t decoded = { 0 };
    td_frame_result_t result = td_frame_decode_slip(wire, len, body, TD_FRAME_BODY_MAX, &decoded);
    assert(len && result == TD_FRAME_OK);
    (void)result;
    return 8 + (size_t)decoded.len;
}

/* Delivers one request on a carrier and plays one link task turn: the request, then any events due. */
static void deliver(void *carrier, const char *json, uint8_t seq)
{
    uint8_t body[TD_FRAME_BODY_MAX];
    size_t len = request_body(json, seq, body);
    if (carrier == g_usb_context) {
        g_usb_handler(body, len, g_usb_context);
    } else {
        td_link_carrier_receive(body, len, carrier);
    }
    handle_frame(0);
    send_events();
}

/* Delivers one chunk or ack frame on a carrier and plays one link task turn. */
static void deliver_frame(void *carrier, uint8_t type, uint8_t flags, const uint8_t *payload, size_t len)
{
    uint8_t wire[TD_FRAME_WIRE_MAX];
    uint8_t body[TD_FRAME_BODY_MAX];
    td_frame_t frame = { .type = type, .seq = 9, .flags = flags, .len = (uint16_t)len, .payload = payload };
    const size_t wire_len = td_frame_encode(&frame, wire, sizeof wire);
    td_frame_t decoded = { 0 };
    td_frame_result_t result = td_frame_decode_slip(wire, wire_len, body, sizeof body, &decoded);
    assert(wire_len && result == TD_FRAME_OK);
    (void)result;
    if (carrier == g_usb_context) {
        g_usb_handler(body, 8 + (size_t)decoded.len, g_usb_context);
    } else {
        td_link_carrier_receive(body, 8 + (size_t)decoded.len, carrier);
    }
    handle_frame(0);
}

static void over_usb(const char *json, uint8_t seq)
{
    deliver(g_usb_context, json, seq);
}

static void over_fake(const char *json, uint8_t seq)
{
    deliver(&g_fake.carrier, json, seq);
}

/* Takes the oldest frame the USB console wrote, SLIP removed. False when it wrote none. */
static bool usb_take(td_frame_t *frame, char *payload)
{
    static uint8_t body[TD_FRAME_BODY_MAX];
    if (!g_usb_len) {
        return false;
    }
    size_t end = 1;
    while (end < g_usb_len && g_usb_wire[end] != TD_FRAME_END) {
        end++;
    }
    assert(end < g_usb_len);
    bool ok = td_frame_decode_slip(g_usb_wire, end + 1, body, sizeof body, frame) == TD_FRAME_OK;
    memmove(g_usb_wire, g_usb_wire + end + 1, g_usb_len - end - 1);
    g_usb_len -= end + 1;
    if (ok) {
        memcpy(payload, frame->payload, frame->len);
        payload[frame->len] = '\0';
    }
    return ok;
}

/* True when the payload has this JSON member, written as cJSON prints it unformatted. */
static bool has(const char *payload, const char *member)
{
    return strstr(payload, member) != NULL;
}

/* ---- Tests ---- */

/* A carrier is added once, behind USB, and only after the link started. */
static void test_carrier_add_rules(void)
{
    reset();
    CHECK(td_link_carrier_add(&g_fake.carrier) == ESP_ERR_INVALID_STATE);
    CHECK(td_link_carrier_add(NULL) == ESP_ERR_INVALID_ARG);
    start(true);
    CHECK(td_link_carrier_add(&g_fake.carrier) == ESP_ERR_INVALID_STATE);
    CHECK(!strcmp(td_link_state().carrier, "usb"));
    static fake_carrier_t second, third;
    second.carrier = third.carrier = (td_link_carrier_t){ .send = fake_send };
    CHECK(td_link_carrier_add(&second.carrier) == ESP_OK);
    CHECK(td_link_carrier_add(&third.carrier) == ESP_ERR_NO_MEM);
    CHECK(td_link_start() == ESP_ERR_INVALID_STATE);
}

/* With only USB, a reply is the SLIP frame td_frame_encode makes, written to the console, as before carriers. */
static void test_usb_reply_is_the_same_wire_bytes(void)
{
    start(false);
    over_usb("{\"m\":\"ping\",\"n\":7}", 42);
    uint8_t written[TD_FRAME_WIRE_MAX];
    size_t written_len = g_usb_len;
    memcpy(written, g_usb_wire, written_len);
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    CHECK(usb_take(&frame, payload));
    CHECK(g_usb_len == 0);
    CHECK(frame.type == TD_FRAME_MESSAGE && frame.seq == 42 && frame.flags == TD_FRAME_RESPONSE);
    CHECK(has(payload, "\"m\":\"ping\"") && has(payload, "\"ok\":true") && has(payload, "\"n\":7"));
    uint8_t expected[TD_FRAME_WIRE_MAX];
    td_frame_t reply = { .type = TD_FRAME_MESSAGE, .seq = 42, .flags = TD_FRAME_RESPONSE,
                         .len = (uint16_t)strlen(payload), .payload = (const uint8_t *)payload };
    size_t expected_len = td_frame_encode(&reply, expected, sizeof expected);
    CHECK(expected_len == written_len && !memcmp(expected, written, written_len));
    td_link_state_t state = td_link_state();
    CHECK(state.frames_rx == 1 && state.frames_tx == 1 && state.dropped == 0 && !state.online);
}

/* With only USB, online state follows hello and the 5 second idle as before carriers. */
static void test_usb_online_state(void)
{
    start(false);
    CHECK(!td_link_state().online);
    over_usb("{\"m\":\"hello\"}", 1);
    td_link_state_t state = td_link_state();
    CHECK(state.online && state.last_rx_ms == g_now_ms);
    g_now_ms += TD_LINK_IDLE_MS - 1;
    CHECK(td_link_state().online);
    g_now_ms += 1;
    CHECK(!td_link_state().online);
    /* A frame after the idle does not bring the link back: only a hello does. */
    over_usb("{\"m\":\"ping\"}", 2);
    CHECK(!td_link_state().online);
    over_usb("{\"m\":\"hello\"}", 3);
    CHECK(td_link_state().online);
    g_usb_handler(NULL, 0, g_usb_context);
    CHECK(td_link_state().crc_errors == 1);
}

/* Messages that arrive on the fake carrier get their replies on it, and USB writes nothing. */
static void test_fake_carrier_replies(void)
{
    start(true);
    over_fake("{\"m\":\"ping\",\"n\":1}", 5);
    CHECK(g_fake.frames == 1);
    CHECK(g_fake.last.seq == 5 && g_fake.last.flags == TD_FRAME_RESPONSE && has(g_fake.payload, "\"n\":1"));
    CHECK(!td_link_state().online);

    over_fake("{\"m\":\"hello\",\"proto\":2}", 6);
    CHECK(g_fake.frames == 2 && g_fake.last.seq == 6);
    CHECK(has(g_fake.payload, "\"m\":\"hello\"") && has(g_fake.payload, "\"proto\":2"));
    CHECK(has(g_fake.payload, "\"id\":\"5a5a5a5a5a5a\""));
    CHECK(td_link_state().online);

    over_fake("{\"m\":\"link.state\"}", 7);
    CHECK(g_fake.frames == 3 && has(g_fake.payload, "\"online\":true"));
    CHECK(has(g_fake.payload, "\"frames_rx\":3") && has(g_fake.payload, "\"frames_tx\":2"));

    /* Events go out on it too. */
    td_link_press("main", "tile", "launch");
    send_events();
    CHECK(g_fake.frames == 4 && g_fake.last.flags == TD_FRAME_EVENT && has(g_fake.payload, "\"m\":\"press\""));
    CHECK(g_usb_len == 0);
}

/* A frame the fake carrier does not take is counted as dropped, not sent. */
static void test_fake_carrier_refuses(void)
{
    start(true);
    g_fake.refuse = true;
    over_fake("{\"m\":\"ping\"}", 1);
    td_link_state_t state = td_link_state();
    CHECK(state.frames_tx == 0 && state.dropped == 1);
    CHECK(g_usb_len == 0);
}

/* USB wins events while it is online, but a reply always leaves on the carrier its request came on. When USB goes
 * idle, events go back to the fake carrier, which stayed online. */
static void test_usb_is_preferred(void)
{
    start(true);
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_fake("{\"m\":\"hello\"}", 1);
    CHECK(g_fake.frames == 1 && has(g_fake.payload, "\"carrier\":\"fake\""));
    CHECK(!td_link_usb_online() && !strcmp(td_link_state().carrier, "fake"));

    g_now_ms += 1000;
    over_usb("{\"m\":\"hello\"}", 2);
    CHECK(g_fake.frames == 1);
    CHECK(usb_take(&frame, payload) && frame.seq == 2 && has(payload, "\"m\":\"hello\""));
    CHECK(has(payload, "\"carrier\":\"usb\""));
    CHECK(td_link_usb_online() && !strcmp(td_link_state().carrier, "usb"));

    g_now_ms += 1000;
    over_fake("{\"m\":\"ping\",\"n\":3}", 3);
    CHECK(g_fake.frames == 2 && g_fake.last.seq == 3 && g_fake.last.flags == TD_FRAME_RESPONSE);
    CHECK(g_usb_len == 0);
    td_link_press("main", "tile", "launch");
    send_events();
    CHECK(usb_take(&frame, payload) && frame.flags == TD_FRAME_EVENT && has(payload, "\"m\":\"press\""));
    CHECK(g_fake.frames == 2);
    CHECK(td_link_state().last_rx_ms == g_now_ms - 1000);

    /* USB has been quiet for 5.5 s and the fake carrier for 4.5 s. */
    g_now_ms += 4500;
    CHECK(!td_link_usb_online());
    over_fake("{\"m\":\"ping\",\"n\":4}", 4);
    CHECK(g_fake.frames == 3 && g_fake.last.seq == 4 && has(g_fake.payload, "\"n\":4"));
    CHECK(g_usb_len == 0);
    td_link_state_t state = td_link_state();
    CHECK(state.online && state.last_rx_ms == g_now_ms && !strcmp(state.carrier, "fake"));
    td_link_press("main", "tile", "launch");
    send_events();
    CHECK(g_fake.frames == 4 && g_fake.last.flags == TD_FRAME_EVENT);

    /* And USB takes over again with its next hello. */
    over_usb("{\"m\":\"hello\"}", 5);
    CHECK(usb_take(&frame, payload) && frame.seq == 5);
    CHECK(g_fake.frames == 4);
}

/* A carrier whose connection ended is offline at once, without waiting for the idle time. */
static void test_carrier_offline(void)
{
    start(true);
    over_fake("{\"m\":\"hello\"}", 1);
    CHECK(td_link_state().online);
    td_link_carrier_offline(&g_fake.carrier);
    CHECK(!td_link_state().online);
    over_fake("{\"m\":\"ping\"}", 2);
    CHECK(!td_link_state().online && g_fake.last.seq == 2);
}

/* A request that waited in the link's queue while its network session ended is not handled, so nothing it would
 * answer reaches the next session. A reply whose session ends while the request is handled is dropped. Each frame
 * a carrier sends carries its connection's generation. */
static void end_fake_session(const cJSON *request, cJSON *reply)
{
    (void)request;
    td_link_carrier_offline(&g_fake.carrier);
    cJSON_AddBoolToObject(reply, "ended", true);
}

static void test_ended_session_gets_nothing(void)
{
    static const td_link_message_t end[] = { { "t.end", end_fake_session, false } };
    start(true);
    CHECK(td_link_messages_add(end, 1) == ESP_OK);
    over_fake("{\"m\":\"ping\"}", 1);
    CHECK(g_fake.frames == 1 && g_fake.generation == 0);
    CHECK(td_link_carrier_generation(&g_fake.carrier) == 0);

    /* Queued, then the session ends before the link task gets to it. */
    uint8_t body[TD_FRAME_BODY_MAX];
    const size_t len = request_body("{\"m\":\"ping\"}", 2, body);
    td_link_carrier_receive(body, len, &g_fake.carrier);
    td_link_carrier_offline(&g_fake.carrier);
    CHECK(td_link_carrier_generation(&g_fake.carrier) == 1);
    const uint32_t dropped = td_link_state().dropped;
    handle_frame(0);
    CHECK(g_fake.frames == 1 && td_link_state().dropped == dropped + 1);

    /* The next session's requests are answered, under its generation. */
    over_fake("{\"m\":\"ping\"}", 3);
    CHECK(g_fake.frames == 2 && g_fake.last.seq == 3 && g_fake.generation == 1);

    /* The session ends while its request is handled: the reply stays home. */
    over_fake("{\"m\":\"t.end\"}", 4);
    CHECK(g_fake.frames == 2 && td_link_state().dropped == dropped + 2);
    CHECK(g_usb_len == 0);
}

/* ---- Transfers ---- */

#define CHUNK TD_FRAME_CHUNK_DATA_MAX

static uint32_t number_arg(const cJSON *request, const char *name)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(request, name);
    return cJSON_IsNumber(item) ? (uint32_t)item->valuedouble : 0;
}

/* The file handlers (td_link_files.c) open a transfer with these calls after they check a request. These handlers
 * take the same steps without the Layout, store, update and Asset checks, which need the store and the OTA slots. */
static void t_put(const cJSON *request, cJSON *reply)
{
    td_transfer_t *t = td_link_transfer();
    const uint32_t bytes = number_arg(request, "bytes");
    if (!td_transfer_begin_buffered(t, TD_TRANSFER_LAYOUT_PUT, bytes)) {
        td_link_fail(reply, "no memory");
        return;
    }
    td_transfer_arm(t, false, bytes, number_arg(request, "crc32"));
    cJSON_AddNumberToObject(reply, "xfer", t->x.id);
}

static void t_get(const cJSON *request, cJSON *reply)
{
    td_transfer_t *t = td_link_transfer();
    const uint32_t bytes = number_arg(request, "bytes");
    if (!td_transfer_begin_buffered(t, TD_TRANSFER_GET, bytes)) {
        td_link_fail(reply, "no memory");
        return;
    }
    for (uint32_t i = 0; i < bytes; i++) {
        t->buf[i] = (uint8_t)(i * 7);
    }
    td_transfer_arm(t, true, bytes, td_xfer_crc32(t->buf, bytes));
    cJSON_AddNumberToObject(reply, "xfer", t->x.id);
}

static void t_update(const cJSON *request, cJSON *reply)
{
    td_transfer_t *t = td_link_transfer();
    td_transfer_begin_streamed(t, TD_TRANSFER_UPDATE);
    td_transfer_arm(t, false, number_arg(request, "bytes"), 0);
    cJSON_AddNumberToObject(reply, "xfer", t->x.id);
}

static void t_asset(const cJSON *request, cJSON *reply)
{
    td_transfer_t *t = td_link_transfer();
    if (t->kind != TD_TRANSFER_NONE) {
        td_link_fail(reply, "busy");
        return;
    }
    td_transfer_begin_streamed(t, TD_TRANSFER_ASSET_PUT);
    td_transfer_arm(t, false, number_arg(request, "bytes"), 0);
    cJSON_AddNumberToObject(reply, "xfer", t->x.id);
}

static void t_photo(const cJSON *request, cJSON *reply)
{
    td_transfer_t *t = td_link_transfer();
    const uint32_t bytes = number_arg(request, "bytes");
    if (!td_transfer_begin_buffered(t, TD_TRANSFER_PHOTO_PUT, bytes)) {
        td_link_fail(reply, "no memory");
        return;
    }
    td_transfer_arm(t, false, bytes, 0);
    t->photo_entry = fx.idle_entry;
    cJSON_AddNumberToObject(reply, "xfer", t->x.id);
}

/* xfer.end's front half: another connection's transfer and a wrong id find nothing, and the rest closes. */
static void t_end(const cJSON *request, cJSON *reply)
{
    td_transfer_t *t = td_link_transfer();
    if (td_transfer_other_origin(t) || t->kind == TD_TRANSFER_NONE || number_arg(request, "xfer") != t->x.id) {
        td_link_fail(reply, "no transfer with that xfer");
        return;
    }
    td_transfer_close(t);
}

static const td_link_message_t k_transfer_messages[] = {
    { "t.put", t_put, false },       { "t.get", t_get, false },     { "t.update", t_update, false },
    { "t.asset", t_asset, false },   { "t.photo", t_photo, false }, { "t.end", t_end, false },
};

static void start_transfers(void)
{
    start(true);
    CHECK(td_link_messages_add(k_transfer_messages, sizeof k_transfer_messages / sizeof k_transfer_messages[0])
          == ESP_OK);
}

/* Opens a transfer with `message` over a carrier, for `bytes`, and returns its xfer id. */
static uint8_t open_transfer(void *carrier, const char *message, uint32_t bytes, uint32_t crc32)
{
    char json[96];
    snprintf(json, sizeof json, "{\"m\":\"%s\",\"bytes\":%u,\"crc32\":%u}", message, (unsigned)bytes,
             (unsigned)crc32);
    deliver(carrier, json, 1);
    return td_link_transfer()->x.id;
}

static void end_transfer(void *carrier, uint8_t xfer)
{
    char json[64];
    snprintf(json, sizeof json, "{\"m\":\"t.end\",\"xfer\":%u}", (unsigned)xfer);
    deliver(carrier, json, 2);
}

static void send_chunk_frame(void *carrier, uint8_t xfer, uint16_t idx, const uint8_t *data, size_t len)
{
    uint8_t payload[TD_FRAME_PAYLOAD_MAX];
    payload[0] = xfer;
    payload[1] = idx & 0xff;
    payload[2] = idx >> 8;
    memcpy(payload + TD_FRAME_CHUNK_HEADER, data, len);
    deliver_frame(carrier, TD_FRAME_CHUNK, 0, payload, TD_FRAME_CHUNK_HEADER + len);
}

static void send_ack_frame(void *carrier, uint8_t xfer, uint16_t next)
{
    const uint8_t payload[3] = { xfer, next & 0xff, next >> 8 };
    deliver_frame(carrier, TD_FRAME_ACK, TD_FRAME_RESPONSE, payload, sizeof payload);
}

/* The service half of a link task turn, as link_task runs it after the frame it handled. */
static void link_turn(void)
{
    td_link_files_service((uint32_t)g_now_ms);
}

/* The next index the last ACK on the fake carrier asked for. */
static unsigned last_ack_next(void)
{
    return g_fake.last.payload[1] | (g_fake.last.payload[2] << 8);
}

static uint8_t g_data[3 * CHUNK];

static void fill_data(void)
{
    for (size_t i = 0; i < sizeof g_data; i++) {
        g_data[i] = (uint8_t)(i * 31 + 5);
    }
}

/* A transfer moves only on the carrier connection that opened it: chunks and acks from another carrier, or from a
 * later session on the same carrier, never reach it, and it is cut when its session ends. */
static void test_transfer_belongs_to_its_session(void)
{
    td_transfer_t *t;
    start_transfers();
    fill_data();
    t = td_link_transfer();
    uint32_t dropped = td_link_state().dropped;
    send_chunk_frame(g_usb_context, 1, 0, g_data, 1);
    CHECK(t->kind == TD_TRANSFER_NONE && td_link_state().dropped == dropped + 1 && g_usb_len == 0);

    /* Opened over USB: the network session's chunks and acks are dropped, USB's go through. */
    uint8_t xfer = open_transfer(g_usb_context, "t.put", CHUNK + 1, 0);
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT);
    g_usb_len = 0;
    dropped = td_link_state().dropped;
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    send_ack_frame(&g_fake.carrier, xfer, 1);
    CHECK(t->x.received == 0 && g_fake.frames == 0 && td_link_state().dropped == dropped + 2);
    send_chunk_frame(g_usb_context, xfer, 0, g_data, CHUNK);
    CHECK(t->x.received == CHUNK && g_usb_len > 0);
    g_usb_len = 0;

    /* Opened over the network: USB's chunks and acks are dropped, and once the session ends the transfer is cut and
     * the next session's frames are dropped too. */
    xfer = open_transfer(&g_fake.carrier, "t.get", CHUNK / 2, 0);
    CHECK(t->kind == TD_TRANSFER_GET && fx.frees == 1);
    link_turn();
    CHECK(g_fake.chunks == 1);
    send_ack_frame(g_usb_context, xfer, 1);
    CHECK(t->x.next == 0);
    send_ack_frame(&g_fake.carrier, xfer, 1);
    CHECK(t->x.next == 1);
    td_link_carrier_offline(&g_fake.carrier);
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.frees == fx.allocs);
    dropped = td_link_state().dropped;
    send_ack_frame(&g_fake.carrier, xfer, 1);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, 1);
    CHECK(td_link_state().dropped == dropped + 2);
}

/* A put takes its chunks in order, answers a duplicate and a gap with the next index it wants, and gives its buffer
 * back when it ends. */
static void test_put_chunks_and_end(void)
{
    start_transfers();
    fill_data();
    td_transfer_t *t = td_link_transfer();
    const uint32_t bytes = 2 * CHUNK + 10;
    const uint8_t xfer = open_transfer(&g_fake.carrier, "t.put", bytes, td_xfer_crc32(g_data, bytes));
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT && fx.allocs == 1 && fx.frees == 0 && xfer != 0);
    CHECK(has(g_fake.payload, "\"ok\":true"));

    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(g_fake.acks == 1 && g_fake.last.type == TD_FRAME_ACK && g_fake.last.flags == TD_FRAME_RESPONSE);
    CHECK(g_fake.last.payload[0] == xfer && last_ack_next() == 1);
    /* A gap and a duplicate. */
    send_chunk_frame(&g_fake.carrier, xfer, 2, g_data + 2 * CHUNK, 10);
    CHECK(g_fake.acks == 2 && last_ack_next() == 1);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(g_fake.acks == 3 && last_ack_next() == 1);
    send_chunk_frame(&g_fake.carrier, xfer, 1, g_data + CHUNK, CHUNK);
    send_chunk_frame(&g_fake.carrier, xfer, 2, g_data + 2 * CHUNK, 10);
    CHECK(g_fake.acks == 5 && last_ack_next() == 3);
    CHECK(t->x.received == bytes && td_xfer_complete(&t->x) && memcmp(t->buf, g_data, bytes) == 0);
    /* The wrong xfer id gets no ack. */
    const uint32_t dropped = td_link_state().dropped;
    send_chunk_frame(&g_fake.carrier, (uint8_t)(xfer + 1), 0, g_data, 1);
    CHECK(g_fake.acks == 5 && td_link_state().dropped == dropped + 1);

    end_transfer(&g_fake.carrier, xfer);
    CHECK(has(g_fake.payload, "\"ok\":true"));
    CHECK(t->kind == TD_TRANSFER_NONE && t->buf == NULL && fx.allocs == 1 && fx.frees == 1);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(g_fake.acks == 5);
}

/* The connection that opened a put ends: the next link turn gives the buffer back, and nothing reaches the next
 * session. A transfer a USB request opened outlives a network session. */
static void test_cut_connection_releases_a_put(void)
{
    start_transfers();
    fill_data();
    td_transfer_t *t = td_link_transfer();
    const uint8_t xfer = open_transfer(&g_fake.carrier, "t.put", CHUNK + 5, 0);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT && t->fill == CHUNK && fx.allocs == 1 && fx.frees == 0);
    link_turn();
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT);

    td_link_carrier_offline(&g_fake.carrier);
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && t->buf == NULL && t->fill == 0 && fx.frees == 1);
    CHECK(fx.update_cancels == 0 && fx.asset_aborts == 0);
    const int acks = g_fake.acks;
    const uint32_t dropped = td_link_state().dropped;
    send_chunk_frame(&g_fake.carrier, xfer, 1, g_data, 5);
    CHECK(g_fake.acks == acks && td_link_state().dropped == dropped + 1);
    link_turn();
    CHECK(fx.frees == 1);

    /* Opened over USB, it survives the network session ending. */
    open_transfer(g_usb_context, "t.put", CHUNK + 5, 0);
    g_usb_len = 0;
    td_link_carrier_offline(&g_fake.carrier);
    link_turn();
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT && fx.allocs == 2 && fx.frees == 1);
}

/* A firmware update that was still receiving is cancelled when its connection ends, and is not when it finished. */
static void test_cut_connection_cancels_an_update(void)
{
    start_transfers();
    fill_data();
    td_transfer_t *t = td_link_transfer();
    uint8_t xfer = open_transfer(&g_fake.carrier, "t.update", CHUNK + 3, 0);
    CHECK(t->kind == TD_TRANSFER_UPDATE && fx.allocs == 0 && t->buf == NULL);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(fx.update_bytes == CHUNK && last_ack_next() == 1);
    td_link_carrier_offline(&g_fake.carrier);
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.update_cancels == 1 && fx.frees == 0);
    link_turn();
    CHECK(fx.update_cancels == 1);

    /* A refused chunk is not taken, and not acked past. */
    fx.update_refuse = true;
    xfer = open_transfer(&g_fake.carrier, "t.update", CHUNK, 0);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(fx.update_bytes == CHUNK && last_ack_next() == 0 && t->x.received == 0);
    fx.update_refuse = false;

    /* The image arrived and update.end took it: dropping the transfer is not a cancel. */
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(td_xfer_complete(&t->x) || t->x.received == CHUNK);
    td_transfer_drop(t);
    CHECK(t->kind == TD_TRANSFER_NONE && fx.update_cancels == 1);
}

/* An Asset upload is aborted when its connection ends, and when its sink refuses a chunk, which records why. */
static void test_asset_upload_is_aborted(void)
{
    start_transfers();
    fill_data();
    td_transfer_t *t = td_link_transfer();
    uint8_t xfer = open_transfer(&g_fake.carrier, "t.asset", CHUNK + 1, 0);
    CHECK(t->kind == TD_TRANSFER_ASSET_PUT && fx.allocs == 0);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(fx.asset_bytes == CHUNK && last_ack_next() == 1);
    td_link_carrier_offline(&g_fake.carrier);
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.asset_aborts == 1);

    fx.asset_refuse = "storage";
    xfer = open_transfer(&g_fake.carrier, "t.asset", CHUNK, 0);
    const int acks = g_fake.acks;
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(g_fake.acks == acks && t->kind == TD_TRANSFER_NONE && fx.asset_aborts == 2);
    CHECK(t->asset_end.xfer == xfer && t->asset_end.reason != NULL && strcmp(t->asset_end.reason, "storage") == 0);
    fx.asset_refuse = NULL;

    /* The next bulk open ends the replay of that end. */
    open_transfer(&g_fake.carrier, "t.put", CHUNK, 0);
    CHECK(t->asset_end.xfer == 0 && t->photo_end.xfer == 0);
}

/* A transfer that stops moving is cut after two seconds, and an update cancelled with it. */
static void test_idle_transfer_is_cut(void)
{
    start_transfers();
    fill_data();
    td_transfer_t *t = td_link_transfer();
    uint8_t xfer = open_transfer(&g_fake.carrier, "t.put", CHUNK + 5, 0);
    g_now_ms += TD_XFER_IDLE_MS;
    link_turn();
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT);
    /* A chunk is activity. */
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    g_now_ms += TD_XFER_IDLE_MS;
    link_turn();
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT);
    g_now_ms += 1;
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.frees == 1 && fx.update_cancels == 0);

    open_transfer(&g_fake.carrier, "t.update", CHUNK, 0);
    g_now_ms += TD_XFER_IDLE_MS + 1;
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.update_cancels == 1);
}

/* A get sends its chunks from the link task, resends the ones nobody acked, and is cut when its connection ends,
 * it is abandoned, or it ran out of retransmissions. */
static void test_get_sends_and_is_cut(void)
{
    start_transfers();
    td_transfer_t *t = td_link_transfer();
    const uint32_t bytes = 2 * CHUNK + 7;
    uint8_t xfer = open_transfer(&g_fake.carrier, "t.get", bytes, 0);
    CHECK(t->kind == TD_TRANSFER_GET && t->x.sending && fx.allocs == 1);
    const int sent = g_fake.frames;
    link_turn();
    CHECK(g_fake.chunks == 3 && g_fake.frames == sent + 3);
    CHECK(g_fake.last.type == TD_FRAME_CHUNK && g_fake.last.payload[0] == xfer && g_fake.last.payload[1] == 2);
    CHECK(g_fake.last.len == TD_FRAME_CHUNK_HEADER + 7);
    CHECK(g_fake.last.payload[TD_FRAME_CHUNK_HEADER] == (uint8_t)(2 * CHUNK * 7));

    /* Nothing acked: all three go again after the retry time. */
    g_now_ms += TD_XFER_RETRY_MS;
    link_turn();
    CHECK(g_fake.chunks == 6);
    send_ack_frame(&g_fake.carrier, xfer, 3);
    CHECK(t->x.next == 3 && td_xfer_complete(&t->x));
    g_now_ms += TD_XFER_RETRY_MS;
    link_turn();
    CHECK(g_fake.chunks == 6 && t->kind == TD_TRANSFER_GET);
    /* The Companion that stops talking leaves the buffer open for 5 seconds after its last ack. */
    g_now_ms += 5000 - TD_XFER_RETRY_MS;
    link_turn();
    CHECK(t->kind == TD_TRANSFER_GET);
    g_now_ms += 1;
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.frees == 1);

    /* Its connection ends before it sends: nothing goes out, and the buffer is freed. */
    open_transfer(&g_fake.carrier, "t.get", bytes, 0);
    const int chunks = g_fake.chunks;
    td_link_carrier_offline(&g_fake.carrier);
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && g_fake.chunks == chunks && fx.frees == 2);

    /* Never acked, it stops after five transmissions of each chunk. */
    open_transfer(&g_fake.carrier, "t.get", CHUNK, 0);
    for (int i = 0; i < 6 && t->kind == TD_TRANSFER_GET; i++) {
        link_turn();
        g_now_ms += TD_XFER_RETRY_MS;
    }
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.frees == 3 && g_fake.chunks == chunks + TD_XFER_TRANSMISSIONS);
}

/* Opening a transfer cuts the one that was open and gives back what it held. A refused update begin is the
 * handler's to check first, so an update that is receiving is never cancelled by it. */
static void test_a_new_transfer_replaces_the_open_one(void)
{
    start_transfers();
    td_transfer_t *t = td_link_transfer();
    const uint8_t first = open_transfer(&g_fake.carrier, "t.put", CHUNK, 0);
    CHECK(fx.allocs == 1 && fx.frees == 0);
    const uint8_t second = open_transfer(&g_fake.carrier, "t.put", CHUNK, 0);
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT && fx.allocs == 2 && fx.frees == 1 && second != first);

    /* An update begin frees the buffer and does not cancel anything. */
    open_transfer(&g_fake.carrier, "t.update", CHUNK, 0);
    CHECK(t->kind == TD_TRANSFER_UPDATE && fx.frees == 2 && fx.update_cancels == 0);
    /* A put replaces the update, which returns to idle. */
    open_transfer(&g_fake.carrier, "t.put", CHUNK, 0);
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT && fx.update_cancels == 1 && fx.allocs == 3);

    /* No memory: the transfer that was open is gone all the same, and the failure is the handler's reply. */
    fx.fail_alloc = true;
    open_transfer(&g_fake.carrier, "t.put", CHUNK, 0);
    CHECK(has(g_fake.payload, "\"ok\":false") && t->kind == TD_TRANSFER_NONE && fx.frees == 3);
    fx.fail_alloc = false;

    /* Xfer ids run from 1 to 255 and wrap. */
    uint8_t xfer = t->last_id;
    for (int i = 0; i < 300; i++) {
        const uint8_t next = open_transfer(&g_fake.carrier, "t.put", 1, 0);
        CHECK(next == (xfer % 255) + 1 && next != 0);
        xfer = next;
    }
}

/* An end request from another connection finds no transfer, and its chunks are dropped. */
static void test_end_belongs_to_its_connection(void)
{
    start_transfers();
    fill_data();
    td_transfer_t *t = td_link_transfer();
    const uint8_t xfer = open_transfer(g_usb_context, "t.put", CHUNK, 0);
    g_usb_len = 0;
    end_transfer(&g_fake.carrier, xfer);
    CHECK(has(g_fake.payload, "\"ok\":false") && t->kind == TD_TRANSFER_LAYOUT_PUT && fx.frees == 0);
    const uint32_t dropped = td_link_state().dropped;
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(t->x.received == 0 && td_link_state().dropped == dropped + 1);
    end_transfer(g_usb_context, xfer);
    g_usb_len = 0;
    CHECK(t->kind == TD_TRANSFER_NONE && fx.frees == 1);
}

/* Selecting a protocol discards an open v2 Asset or picture upload, and leaves a put and an update alone. */
static void test_protocol_selection_drops_only_v2_uploads(void)
{
    start_transfers();
    td_transfer_t *t = td_link_transfer();
    open_transfer(&g_fake.carrier, "t.put", CHUNK, 0);
    td_link_session_reset();
    CHECK(t->kind == TD_TRANSFER_LAYOUT_PUT && fx.frees == 0);
    open_transfer(&g_fake.carrier, "t.update", CHUNK, 0);
    td_link_session_reset();
    CHECK(t->kind == TD_TRANSFER_UPDATE && fx.update_cancels == 0);
    td_transfer_close(t);
    fx.update_cancels = 0;

    open_transfer(&g_fake.carrier, "t.asset", CHUNK, 0);
    td_link_session_reset();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.asset_aborts == 1);
    fx.idle_face = true;
    open_transfer(&g_fake.carrier, "t.photo", CHUNK, 0);
    const int frees = fx.frees;
    td_link_session_reset();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.frees == frees + 1);
}

/* A picture put lasts while the idle face it was admitted under does: the next chunk, or the next link turn, cuts
 * it and records that it was cut for "idle". */
static void test_picture_put_follows_the_idle_face(void)
{
    start_transfers();
    fill_data();
    td_transfer_t *t = td_link_transfer();
    fx.idle_face = true;
    fx.idle_entry = 7;
    uint8_t xfer = open_transfer(&g_fake.carrier, "t.photo", CHUNK + 1, 0);
    send_chunk_frame(&g_fake.carrier, xfer, 0, g_data, CHUNK);
    CHECK(t->kind == TD_TRANSFER_PHOTO_PUT && last_ack_next() == 1);

    /* The face came up again: a different entry. */
    fx.idle_entry = 8;
    const int acks = g_fake.acks;
    send_chunk_frame(&g_fake.carrier, xfer, 1, g_data, 1);
    CHECK(g_fake.acks == acks && t->kind == TD_TRANSFER_NONE && fx.frees == 1);
    CHECK(t->photo_end.xfer == xfer && strcmp(t->photo_end.failure, "idle") == 0);

    /* The Device woke up between chunks. */
    xfer = open_transfer(&g_fake.carrier, "t.photo", CHUNK + 1, 0);
    CHECK(t->kind == TD_TRANSFER_PHOTO_PUT && t->photo_end.xfer == 0);
    fx.idle_face = false;
    link_turn();
    CHECK(t->kind == TD_TRANSFER_NONE && fx.frees == 2);
    CHECK(t->photo_end.xfer == xfer && strcmp(t->photo_end.failure, "idle") == 0);
}

static void net_state(const cJSON *request, cJSON *reply)
{
    (void)request;
    cJSON_AddStringToObject(reply, "state", "idle");
}

static void net_pair(const cJSON *request, cJSON *reply)
{
    (void)request;
    cJSON_AddBoolToObject(reply, "open", true);
}

/* A network carrier carries only the messages net-v2.md allows; the rest answer usb-only and do nothing. Over USB the
 * same messages run. */
static void test_network_allowlist(void)
{
    static const td_link_message_t net[] = {
        { "net.state", net_state, false },
        { "net.pair", net_pair, true },
    };
    start(true);
    g_fake.carrier.network = true;
    CHECK(td_link_messages_add(net, 0) == ESP_ERR_INVALID_ARG);
    CHECK(td_link_messages_add(net, 2) == ESP_OK);
    CHECK(td_link_messages_add(net, 1) == ESP_OK);
    CHECK(td_link_messages_add(net, 1) == ESP_ERR_NO_MEM);

    static const char *const refused[] = {
        "update.begin", "update.end", "net.pair", "net.list", "net.forget", "wifi.set", "wifi.forget", "store.wipe",
        "reboot", "v2.unknown",
    };
    for (size_t i = 0; i < sizeof refused / sizeof refused[0]; i++) {
        char json[64];
        snprintf(json, sizeof json, "{\"m\":\"%s\"}", refused[i]);
        over_fake(json, (uint8_t)(10 + i));
        CHECK(g_fake.last.seq == 10 + i && has(g_fake.payload, "\"ok\":false"));
        CHECK(has(g_fake.payload, "\"reason\":\"usb-only\"") && has(g_fake.payload, "\"error\":\"Only over USB\""));
    }
    over_fake("{\"m\":\"net.state\"}", 30);
    CHECK(has(g_fake.payload, "\"ok\":true") && has(g_fake.payload, "\"state\":\"idle\""));
    over_fake("{\"m\":\"ping\",\"n\":2}", 31);
    CHECK(has(g_fake.payload, "\"ok\":true") && has(g_fake.payload, "\"n\":2"));
    over_fake("{\"m\":\"settings.get\"}", 32);
    CHECK(!has(g_fake.payload, "usb-only"));
    over_fake("{\"m\":\"v2.idle.get\"}", 33);
    CHECK(has(g_fake.payload, "not-negotiated"));

    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_usb("{\"m\":\"net.pair\"}", 40);
    CHECK(usb_take(&frame, payload) && frame.seq == 40 && has(payload, "\"open\":true"));
    over_usb("{\"m\":\"update.begin\"}", 41);
    CHECK(usb_take(&frame, payload) && !has(payload, "usb-only"));
}

/* The SLIP-free encoder makes exactly the body the SLIP encoder wraps. */
static void test_frame_body_encoder(void)
{
    static const uint8_t payload[] = { 0xc0, 0xdb, 'x', 0xc0 };
    const td_frame_t frame = { .type = TD_FRAME_MESSAGE, .seq = 9, .flags = TD_FRAME_RESPONSE, .len = sizeof payload,
                               .payload = payload };
    uint8_t body[TD_FRAME_BODY_MAX], wire[TD_FRAME_WIRE_MAX], decoded[TD_FRAME_BODY_MAX];
    const size_t len = td_frame_encode_body(&frame, body, sizeof body);
    CHECK(len == 8 + sizeof payload);
    CHECK(td_frame_encode_body(&frame, body, len - 1) == 0);
    const size_t wire_len = td_frame_encode(&frame, wire, sizeof wire);
    size_t used = 0;
    for (size_t i = 1; i + 1 < wire_len; i++) {
        decoded[used++] = wire[i] == TD_FRAME_ESC ? (wire[++i] == 0xdc ? TD_FRAME_END : TD_FRAME_ESC) : wire[i];
    }
    CHECK(used == len && !memcmp(decoded, body, len));
    td_frame_t back;
    CHECK(td_frame_decode(body, len, &back) == TD_FRAME_OK && back.seq == 9 && !memcmp(back.payload, payload, 4));
}

/* A hello brings only its own carrier online. When USB goes idle, a carrier that never said hello is offline. */
static void test_online_is_per_carrier(void)
{
    start(true);
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_fake("{\"m\":\"ping\"}", 1);
    over_usb("{\"m\":\"hello\"}", 2);
    CHECK(usb_take(&frame, payload) && frame.seq == 2);
    CHECK(td_link_state().online);

    g_now_ms += 3000;
    over_fake("{\"m\":\"ping\"}", 3);
    CHECK(g_fake.last.seq == 3 && g_usb_len == 0);
    g_now_ms += 3000;
    over_fake("{\"m\":\"link.state\"}", 4);
    CHECK(g_fake.last.seq == 4 && has(g_fake.payload, "\"online\":false"));
    CHECK(!td_link_state().online);
    CHECK(g_usb_len == 0);
}

/* Only the radio build marks its v2 hello with "radio":true. The v1 hello never carries it. This file is compiled
 * twice, once with CONFIG_TD_RADIO_BUILD. */
static void test_hello_marks_the_radio_build(void)
{
    start(false);
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_usb("{\"m\":\"hello\"}", 1);
    CHECK(usb_take(&frame, payload) && !has(payload, "\"radio\""));
    over_usb("{\"m\":\"hello\",\"proto\":2}", 2);
    CHECK(usb_take(&frame, payload) && has(payload, "\"proto\":2"));
#if CONFIG_TD_RADIO_BUILD
    CHECK(has(payload, "\"radio\":true"));
    CHECK(has(payload, "\"bluetooth\""));
#else
    CHECK(!has(payload, "\"radio\""));
    CHECK(!has(payload, "\"bluetooth\""));
#endif
}

/* A frame from a carrier the link does not know is dropped, and nothing answers it. */
static void test_unknown_carrier_dropped(void)
{
    start(false);
    deliver(&g_fake.carrier, "{\"m\":\"ping\"}", 1);
    td_link_state_t state = td_link_state();
    CHECK(state.frames_rx == 0 && state.dropped == 1);
    CHECK(g_fake.frames == 0 && g_usb_len == 0);
}

/* ---- Wi-Fi: the `wifi.*` messages, added with td_link_extend as the radio build does, answer only over USB ---- */

static int g_wifi_calls;
static char g_wifi_last[32];

static void wifi_handler(const cJSON *request, cJSON *reply)
{
    g_wifi_calls++;
    snprintf(g_wifi_last, sizeof g_wifi_last, "%s",
             cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "m")));
    cJSON_AddStringToObject(reply, "state", "off");
}

static void wifi_caps(cJSON *caps)
{
    cJSON_AddBoolToObject(caps, "wifi", true);
}

/* td_radio.c's extension, with a stand-in handler. test_extension_is_usb_only adds it. */
static const td_link_extension_t g_wifi_extension = { .prefix = "wifi.", .handle = wifi_handler, .caps = wifi_caps };

#define WIFI_SET "{\"m\":\"wifi.set\",\"ssid\":\"td-test-nonexistent\",\"password\":\"dummy-pass\"}"

/* Before any extension, as in the normal build: every `wifi.*` message gets the unknown-message error, and hello has
 * no wifi cap. Runs before test_extension_is_usb_only. */
static void test_wifi_unknown_without_extension(void)
{
    start(true);
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_usb("{\"m\":\"hello\",\"proto\":2}", 1);
    CHECK(usb_take(&frame, payload) && has(payload, "\"caps\":") && !has(payload, "\"wifi\""));
    const char *const messages[] = { "{\"m\":\"wifi.state\"}", "{\"m\":\"wifi.forget\"}", WIFI_SET };
    for (size_t i = 0; i < sizeof messages / sizeof messages[0]; i++) {
        over_usb(messages[i], (uint8_t)(2 + i));
        CHECK(usb_take(&frame, payload) && has(payload, "\"ok\":false") && has(payload, "\"error\":\"unknown message\""));
        CHECK(!has(payload, "dummy-pass"));
    }
}

static int g_extension_calls;

static void extension_handle(const cJSON *request, cJSON *reply)
{
    (void)request;
    g_extension_calls++;
    cJSON_AddBoolToObject(reply, "handled", true);
}

static void extension_caps(cJSON *caps)
{
    cJSON_AddNumberToObject(caps, "ble_hid", 4);
}

/* A message family added with td_link_extend runs only over USB: on the fake carrier it is refused with usb-only and
 * its handler never runs. Its caps join the v2 hello on every carrier. */
static void test_extension_is_usb_only(void)
{
    static const td_link_extension_t ble = { .prefix = "ble.", .handle = extension_handle, .caps = extension_caps };
    static const td_link_extension_t bad_prefix = { .prefix = "ble", .handle = extension_handle };
    static const td_link_extension_t v2 = { .prefix = "v2.ble.", .handle = extension_handle };
    static const td_link_extension_t no_handler = { .prefix = "wifi." };
    static const td_link_extension_t third = { .prefix = "net.", .handle = extension_handle };
    /* The extensions it adds stay for the rest of the process, as they do on the Device: the Wi-Fi tests after it use
     * g_wifi_extension. */
    CHECK(td_link_extend(NULL) == ESP_ERR_INVALID_ARG);
    CHECK(td_link_extend(&bad_prefix) == ESP_ERR_INVALID_ARG);
    CHECK(td_link_extend(&v2) == ESP_ERR_INVALID_ARG);
    CHECK(td_link_extend(&no_handler) == ESP_ERR_INVALID_ARG);
    CHECK(td_link_extend(&ble) == ESP_OK);
    CHECK(td_link_extend(&ble) == ESP_ERR_INVALID_STATE);
    CHECK(td_link_extend(&g_wifi_extension) == ESP_OK);
    CHECK(td_link_extend(&third) == ESP_ERR_NO_MEM);

    start(true);
    g_extension_calls = 0;
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_usb("{\"m\":\"ble.state\"}", 1);
    CHECK(usb_take(&frame, payload) && frame.seq == 1);
    CHECK(has(payload, "\"m\":\"ble.state\"") && has(payload, "\"ok\":true") && has(payload, "\"handled\":true"));
    CHECK(g_extension_calls == 1);

    over_fake("{\"m\":\"ble.forget\"}", 2);
    CHECK(g_fake.frames == 1 && g_fake.last.seq == 2);
    CHECK(has(g_fake.payload, "\"ok\":false") && has(g_fake.payload, "\"reason\":\"usb-only\""));
    CHECK(has(g_fake.payload, "\"error\":\"Only over USB\"") && !has(g_fake.payload, "handled"));
    CHECK(g_extension_calls == 1);

    over_fake("{\"m\":\"hello\",\"proto\":2}", 3);
    CHECK(g_fake.frames == 2 && has(g_fake.payload, "\"ble_hid\":4"));
    over_usb("{\"m\":\"hello\"}", 4);
    CHECK(usb_take(&frame, payload) && !has(payload, "ble_hid"));
}

/* After test_extension_is_usb_only added g_wifi_extension. */
static void test_wifi_over_usb_only(void)
{
    start(true);
    g_wifi_calls = 0;
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];

    over_usb("{\"m\":\"hello\",\"proto\":2}", 1);
    CHECK(usb_take(&frame, payload) && has(payload, "\"wifi\":true"));
    /* A v1 hello carries no caps at all. */
    over_usb("{\"m\":\"hello\"}", 2);
    CHECK(usb_take(&frame, payload) && !has(payload, "\"wifi\""));

    over_usb("{\"m\":\"wifi.state\"}", 3);
    CHECK(usb_take(&frame, payload) && frame.seq == 3 && has(payload, "\"m\":\"wifi.state\""));
    CHECK(has(payload, "\"ok\":true") && has(payload, "\"state\":\"off\""));
    CHECK(g_wifi_calls == 1 && !strcmp(g_wifi_last, "wifi.state"));

    /* On any other carrier the handler never runs, so nothing is stored, read or forgotten. USB goes idle first, so
     * the replies leave on the fake carrier; while USB is online they would leave on USB, still refused. */
    g_now_ms += TD_LINK_IDLE_MS;
    const char *const messages[] = { WIFI_SET, "{\"m\":\"wifi.state\"}", "{\"m\":\"wifi.forget\"}",
                                     "{\"m\":\"wifi.scan\"}" };
    for (size_t i = 0; i < sizeof messages / sizeof messages[0]; i++) {
        over_fake(messages[i], (uint8_t)(10 + i));
        CHECK(g_fake.last.seq == (uint8_t)(10 + i) && has(g_fake.payload, "\"ok\":false"));
        CHECK(has(g_fake.payload, "\"reason\":\"usb-only\"") && has(g_fake.payload, "\"error\":\"Only over USB\""));
        CHECK(!has(g_fake.payload, "dummy-pass"));
    }
    CHECK(g_wifi_calls == 1 && g_usb_len == 0);

    /* Over USB the strict JSON rules of link-v2 apply: a control character escaped in a string never reaches the
     * handler. */
    over_usb("{\"m\":\"wifi.set\",\"ssid\":\"a\\nb\",\"password\":\"dummy-pass\"}", 20);
    CHECK(usb_take(&frame, payload) && has(payload, "\"ok\":false") && has(payload, "\"reason\":\"args\""));
    CHECK(g_wifi_calls == 1);
    CHECK(!has(payload, "dummy-pass"));
    over_usb("{\"m\":\"wifi.forget\"}", 21);
    CHECK(usb_take(&frame, payload) && g_wifi_calls == 2 && !strcmp(g_wifi_last, "wifi.forget"));
}

static bool holds(const void *buf, size_t len, const char *text)
{
    const size_t n = strlen(text);
    for (size_t i = 0; i + n <= len; i++) {
        if (!memcmp((const uint8_t *)buf + i, text, n)) {
            return true;
        }
    }
    return false;
}

/* No link buffer holds the password: not the receive packet, not the stream buffer between the carriers and the link
 * task, not the frame body, not the message text. */
static bool link_holds(const char *text)
{
    return holds(s_rx_packet, sizeof s_rx_packet, text) || holds(s_frames_storage, TD_LINK_STREAM_BYTES + 1, text) ||
           holds(s_body, sizeof s_body, text) || holds(s_message, sizeof s_message, text);
}

/* After a `wifi.set`, handled, refused or stale, no link buffer still holds its password. */
static void test_wifi_set_leaves_no_copy(void)
{
    start(true);
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    const char *const sets[] = {
        WIFI_SET,
        /* Refused by the strict JSON rules before the handler. */
        "{\"m\":\"wifi.set\",\"ssid\":\"a\\nb\",\"password\":\"dummy-pass\"}",
    };
    for (size_t i = 0; i < sizeof sets / sizeof sets[0]; i++) {
        over_usb(sets[i], (uint8_t)(30 + i));
        CHECK(usb_take(&frame, payload) && !has(payload, "dummy-pass"));
        CHECK(!link_holds("dummy-pass"));
    }

    /* While another frame waits behind it the stream buffer keeps its bytes; once it drains they are wiped. */
    uint8_t body[TD_FRAME_BODY_MAX];
    size_t len = request_body(WIFI_SET, 40, body);
    g_usb_handler(body, len, g_usb_context);
    len = request_body("{\"m\":\"ping\"}", 41, body);
    g_usb_handler(body, len, g_usb_context);
    CHECK(holds(s_frames_storage, TD_LINK_STREAM_BYTES + 1, "dummy-pass"));
    handle_frame(0);
    CHECK(usb_take(&frame, payload) && frame.seq == 40);
    CHECK(holds(s_frames_storage, TD_LINK_STREAM_BYTES + 1, "dummy-pass") && s_frames_secret);
    CHECK(!holds(s_body, sizeof s_body, "dummy-pass") && !holds(s_message, sizeof s_message, "dummy-pass"));
    handle_frame(0);
    CHECK(usb_take(&frame, payload) && frame.seq == 41);
    CHECK(!link_holds("dummy-pass") && !s_frames_secret);

    /* A `wifi.set` whose connection ended while it waited is never handled, and still leaves nothing. */
    len = request_body(WIFI_SET, 42, body);
    td_link_carrier_receive(body, len, &g_fake.carrier);
    td_link_carrier_offline(&g_fake.carrier);
    const int frames = g_fake.frames;
    handle_frame(0);
    CHECK(g_fake.frames == frames && !link_holds("dummy-pass"));
}

/* The longest v2 hello, with the wifi and ble_hid caps, a full Device name and a full version, still fits one frame. */
static void test_wifi_hello_fits(void)
{
    start(false);
    memset(g_name, 'N', TD_DEVICE_NAME_MAX);
    g_name[TD_DEVICE_NAME_MAX] = '\0';
    memset(g_app.version, 'v', sizeof g_app.version - 1);
    g_app.version[sizeof g_app.version - 1] = '\0';
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_usb("{\"m\":\"hello\",\"proto\":2}", 1);
    CHECK(usb_take(&frame, payload) && has(payload, "\"wifi\":true") && has(payload, "\"ble_hid\":4"));
    CHECK(has(payload, g_name) && frame.len < TD_FRAME_PAYLOAD_MAX && td_link_state().dropped == 0);
    strcpy(g_name, "Open TouchDeck");
    strcpy(g_app.version, "host");
}

/* ---- Device names: the shared vectors in schema/protocol/name-vectors-v2.json ---- */

static cJSON *g_names;

static cJSON *load_names(const char *path)
{
    FILE *file = fopen(path, "rb");
    if (!file) {
        fprintf(stderr, "cannot open %s\n", path);
        exit(2);
    }
    static char text[1 << 16];
    size_t len = fread(text, 1, sizeof text - 1, file);
    fclose(file);
    text[len] = '\0';
    cJSON *json = cJSON_Parse(text);
    if (!json) {
        fprintf(stderr, "cannot parse %s\n", path);
        exit(2);
    }
    return json;
}

static const char *vector_name(const cJSON *item)
{
    return cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(item, "name"));
}

static const char *vector_label(const cJSON *item)
{
    return cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(item, "label"));
}

static uint32_t hex_value(const char *text, const char **end)
{
    return (uint32_t)strtoul(text, (char **)end, 16);
}

static void utf8(uint32_t c, char out[5])
{
    int n = 0;
    if (c < 0x80) out[n++] = (char)c;
    else if (c < 0x800) { out[n++] = (char)(0xc0 | c >> 6); out[n++] = (char)(0x80 | (c & 63)); }
    else if (c < 0x10000) {
        out[n++] = (char)(0xe0 | c >> 12); out[n++] = (char)(0x80 | ((c >> 6) & 63)); out[n++] = (char)(0x80 | (c & 63));
    } else {
        out[n++] = (char)(0xf0 | c >> 18); out[n++] = (char)(0x80 | ((c >> 12) & 63));
        out[n++] = (char)(0x80 | ((c >> 6) & 63)); out[n++] = (char)(0x80 | (c & 63));
    }
    out[n] = '\0';
}

/* The validator takes and refuses the shared names, and refuses every control, every code point of the pinned Cf
 * table and each Hangul filler, and only those. */
static void test_name_vectors_in_the_validator(void)
{
    const cJSON *item;
    cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, "accepted")) {
        if (!td_device_name_valid(vector_name(item))) fprintf(stderr, "accepted vector refused: %s\n", vector_label(item));
        CHECK(td_device_name_valid(vector_name(item)));
    }
    cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, "rejected")) {
        if (td_device_name_valid(vector_name(item))) fprintf(stderr, "rejected vector taken: %s\n", vector_label(item));
        CHECK(!td_device_name_valid(vector_name(item)));
    }
    static unsigned char marked[0x110000];
    static const char *const RANGES[] = { "control_ranges", "format_ranges" };
    for (size_t i = 0; i < sizeof RANGES / sizeof RANGES[0]; i++) {
        cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, RANGES[i])) {
            const char *end;
            uint32_t first = hex_value(item->valuestring, &end);
            uint32_t last = *end == '-' ? hex_value(end + 1, &end) : first;
            for (uint32_t c = first; c <= last; c++) marked[c] = 1;
        }
    }
    cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, "hangul_fillers")) {
        marked[hex_value(item->valuestring, NULL)] = 1;
    }
    for (uint32_t c = 1; c < 0x110000; c++) { /* U+0000 would end the C string */
        if (c >= 0xd800 && c <= 0xdfff) continue; /* surrogates are not UTF-8 */
        char text[16] = "a";
        utf8(c, text + 1);
        strcat(text, "b");
        if (td_device_name_valid(text) == (marked[c] != 0)) {
            fprintf(stderr, "U+%04X: validator and table disagree\n", (unsigned)c);
            g_failures++;
        }
    }
}

/* v2.name.set refuses a vector name with args and keeps its reply shape, and takes an accepted one. */
static void test_name_set_refuses_vectors_with_args(void)
{
    start(true);
    over_fake("{\"m\":\"hello\",\"proto\":2}", 1);
    uint8_t seq = 2;
    const cJSON *item;
    cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, "rejected")) {
        cJSON *request = cJSON_CreateObject();
        cJSON_AddStringToObject(request, "m", "v2.name.set");
        cJSON_AddStringToObject(request, "name", vector_name(item));
        char *text = cJSON_PrintUnformatted(request);
        over_fake(text, seq++);
        if (!(has(g_fake.payload, "\"ok\":false") && has(g_fake.payload, "\"reason\":\"args\""))) {
            fprintf(stderr, "v2.name.set took %s: %s\n", vector_label(item), g_fake.payload);
        }
        CHECK(has(g_fake.payload, "\"ok\":false") && has(g_fake.payload, "\"reason\":\"args\""));
        free(text);
        cJSON_Delete(request);
    }
    cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, "accepted")) {
        cJSON *request = cJSON_CreateObject();
        cJSON_AddStringToObject(request, "m", "v2.name.set");
        cJSON_AddStringToObject(request, "name", vector_name(item));
        char *text = cJSON_PrintUnformatted(request);
        over_fake(text, seq++);
        CHECK(has(g_fake.payload, "\"ok\":true"));
        free(text);
        cJSON_Delete(request);
    }
}

int main(int argc, char **argv)
{
    if (argc != 2) {
        fprintf(stderr, "usage: %s name-vectors-v2.json\n", argv[0]);
        return 2;
    }
    g_names = load_names(argv[1]);
    test_carrier_add_rules();
    test_usb_reply_is_the_same_wire_bytes();
    test_usb_online_state();
    test_fake_carrier_replies();
    test_fake_carrier_refuses();
    test_usb_is_preferred();
    test_online_is_per_carrier();
    test_unknown_carrier_dropped();
    test_carrier_offline();
    test_ended_session_gets_nothing();
    test_transfer_belongs_to_its_session();
    test_put_chunks_and_end();
    test_cut_connection_releases_a_put();
    test_cut_connection_cancels_an_update();
    test_asset_upload_is_aborted();
    test_idle_transfer_is_cut();
    test_get_sends_and_is_cut();
    test_a_new_transfer_replaces_the_open_one();
    test_end_belongs_to_its_connection();
    test_protocol_selection_drops_only_v2_uploads();
    test_picture_put_follows_the_idle_face();
    test_network_allowlist();
    test_frame_body_encoder();
    test_name_vectors_in_the_validator();
    test_name_set_refuses_vectors_with_args();
    test_wifi_unknown_without_extension();
    test_extension_is_usb_only();
    test_hello_marks_the_radio_build();
    /* The rest run with the `ble.` and `wifi.` extensions it added, as the radio build does. */
    test_wifi_over_usb_only();
    test_wifi_hello_fits();
    test_wifi_set_leaves_no_copy();
    reset();
    if (g_failures) {
        fprintf(stderr, "%d checks failed\n", g_failures);
        return 1;
    }
    printf("link carrier%s: 31 tests passed\n", CONFIG_TD_RADIO_BUILD ? " (radio build)" : "");
    return 0;
}
