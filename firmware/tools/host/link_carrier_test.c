/*
 * The link's carrier seam on the host. The production td_link.c, its message dispatcher td_link_msgs.c, the frame codec
 * and cJSON run unchanged. Frames arrive through the USB console's frame handler, as the console task delivers them,
 * and through a fake carrier. The test plays the link task's turns and checks which carrier each reply and event
 * leaves on, and what `link.state` reports. td_link.c is included so the test can take those turns one at a time.
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
};

StreamBufferHandle_t xStreamBufferCreate(size_t size, size_t trigger)
{
    (void)trigger;
    struct host_stream *s = calloc(1, sizeof *s);
    s->buf = malloc(size);
    s->cap = size;
    return s;
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
    free(s->buf);
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

void td_link_files_chunk(uint8_t seq, const uint8_t *payload, size_t len) { (void)seq; (void)payload; (void)len; }
void td_link_files_ack(const uint8_t *payload, size_t len) { (void)payload; (void)len; }
void td_link_files_service(uint32_t now_ms) { (void)now_ms; }
void td_link_files_select(void) {}

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

const esp_app_desc_t *esp_app_get_description(void)
{
    static const esp_app_desc_t app = { .version = "host" };
    return &app;
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

esp_err_t td_device_name_get(char out[TD_DEVICE_NAME_MAX + 1]) { strcpy(out, "Open TouchDeck"); return ESP_OK; }
esp_err_t td_device_name_set(const char *name) { (void)name; return ESP_OK; }
bool td_recovery_mode(void) { return false; }

/* ---- The fake carrier ---- */

typedef struct {
    td_link_carrier_t carrier;
    bool refuse;
    int frames;
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
    s_frames = NULL;
    s_presses = NULL;
    s_rx_mutex = NULL;
    memset(&s_state, 0, sizeof s_state);
    memset(s_carriers, 0, sizeof s_carriers);
    s_carrier_count = 0;
    s_current = 0;
    g_usb_len = 0;
    g_usb_handler = NULL;
    g_usb_context = NULL;
    memset(&g_fake, 0, sizeof g_fake);
    g_fake.carrier = (td_link_carrier_t){ .send = fake_send };
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

/* USB wins while it is online. When it goes idle the link goes back to the fake carrier, which stayed online. */
static void test_usb_is_preferred(void)
{
    start(true);
    td_frame_t frame;
    char payload[TD_FRAME_PAYLOAD_MAX + 1];
    over_fake("{\"m\":\"hello\"}", 1);
    CHECK(g_fake.frames == 1);

    g_now_ms += 1000;
    over_usb("{\"m\":\"hello\"}", 2);
    CHECK(g_fake.frames == 1);
    CHECK(usb_take(&frame, payload) && frame.seq == 2 && has(payload, "\"m\":\"hello\""));

    g_now_ms += 1000;
    over_fake("{\"m\":\"ping\",\"n\":3}", 3);
    CHECK(g_fake.frames == 1);
    CHECK(usb_take(&frame, payload) && frame.seq == 3);
    td_link_press("main", "tile", "launch");
    send_events();
    CHECK(usb_take(&frame, payload) && frame.flags == TD_FRAME_EVENT && has(payload, "\"m\":\"press\""));
    CHECK(td_link_state().last_rx_ms == g_now_ms - 1000);

    /* USB has been quiet for 5.5 s and the fake carrier for 4.5 s. */
    g_now_ms += 4500;
    over_fake("{\"m\":\"ping\",\"n\":4}", 4);
    CHECK(g_fake.frames == 2 && g_fake.last.seq == 4 && has(g_fake.payload, "\"n\":4"));
    CHECK(g_usb_len == 0);
    td_link_state_t state = td_link_state();
    CHECK(state.online && state.last_rx_ms == g_now_ms);

    /* And USB takes over again with its next hello. */
    over_usb("{\"m\":\"hello\"}", 5);
    CHECK(usb_take(&frame, payload) && frame.seq == 5);
    CHECK(g_fake.frames == 2);
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
    CHECK(usb_take(&frame, payload) && frame.seq == 3);
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
#else
    CHECK(!has(payload, "\"radio\""));
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

/* The validator takes and refuses the shared names, and refuses every code point of the pinned Cf table and each
 * Hangul filler, and only those. */
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
    cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, "format_ranges")) {
        const char *end;
        uint32_t first = hex_value(item->valuestring, &end);
        uint32_t last = *end == '-' ? hex_value(end + 1, &end) : first;
        for (uint32_t c = first; c <= last; c++) marked[c] = 1;
    }
    cJSON_ArrayForEach(item, cJSON_GetObjectItemCaseSensitive(g_names, "hangul_fillers")) {
        marked[hex_value(item->valuestring, NULL)] = 1;
    }
    for (uint32_t c = 0x20; c < 0x110000; c++) {
        if (c == 0x7f || (c >= 0xd800 && c <= 0xdfff)) continue; /* DEL and surrogates have their own rules */
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
    test_name_vectors_in_the_validator();
    test_name_set_refuses_vectors_with_args();
    test_hello_marks_the_radio_build();
    reset();
    if (g_failures) {
        fprintf(stderr, "%d checks failed\n", g_failures);
        return 1;
    }
    printf("link carrier%s: 11 tests passed\n", CONFIG_TD_RADIO_BUILD ? " (radio build)" : "");
    return 0;
}
