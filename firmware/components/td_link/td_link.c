#include "td_link.h"
#include "td_link_carrier.h"
#include "td_link_priv.h"
#include "key_sink_link.h"
#include "td_frame.h"
#include "td_v2_json.h"
#include "td_rx_sync.h"
#include "td_input.h"
#include "ui.h"
#include "ui_page_report.h"
#include <stdio.h>
#include <string.h>
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "freertos/stream_buffer.h"
#include "freertos/task.h"

#define TD_LINK_STREAM_BYTES 8192
#define TD_LINK_IDLE_MS 5000
#define TD_LINK_POLL_MS 10
#define TD_LINK_PRESS_QUEUE 16
#define TD_LINK_EVENTS_PER_TURN 16
#define TD_LINK_PACKET_HEADER 7

_Static_assert(TD_RX_FRAME_MAX >= TD_FRAME_BODY_MAX, "receive buffer must hold a decoded maximum frame");

/* One carrier's online state. A carrier is online from a hello on it until nothing arrived on it for TD_LINK_IDLE_MS.
 * Its generation counts the connections it reported gone (td_link_carrier_offline), so a frame, a reply or a transfer
 * of an ended network session never reaches the next one. */
typedef struct {
    const td_link_carrier_t *carrier;
    bool online;
    int64_t last_rx_ms;
    uint32_t generation;
} carrier_slot_t;

static StreamBufferHandle_t s_frames;
static SemaphoreHandle_t s_rx_mutex;
static portMUX_TYPE s_lock = portMUX_INITIALIZER_UNLOCKED;
/* The counters. Its online and last_rx_ms are the active carrier's, which td_link_state fills in. */
static td_link_state_t s_state;
static carrier_slot_t s_carriers[TD_LINK_CARRIERS_MAX];
static size_t s_carrier_count;
/* The slot of the frame the link task handles now, or handled last, and that carrier's generation when it arrived. */
static size_t s_current;
static uint32_t s_current_generation;
/* The origin of the open transfer (td_link_transfer_claim), or 0. */
static uint32_t s_transfer;
static const td_link_message_t *s_tables[TD_LINK_MESSAGE_TABLES];
static size_t s_table_counts[TD_LINK_MESSAGE_TABLES];
/* A packet in s_frames is the slot, the body length, the carrier's generation and the body. One receive at a time fills this, under s_rx_mutex. */
static uint8_t s_rx_packet[TD_LINK_PACKET_HEADER + TD_FRAME_BODY_MAX];
static QueueHandle_t s_presses;
static uint32_t s_press_seq;
static td_input_mailbox_t s_input;

typedef td_press_t press_t;

/* Call under s_lock. */
static bool slot_online(size_t slot, int64_t now)
{
    return s_carriers[slot].online && now - s_carriers[slot].last_rx_ms < TD_LINK_IDLE_MS;
}

/* The first carrier added that is online, else the one the last handled frame came on. Call under s_lock, with a
 * carrier added. */
static size_t active_slot(int64_t now)
{
    for (size_t slot = 0; slot < s_carrier_count; slot++) {
        if (slot_online(slot, now)) {
            return slot;
        }
    }
    return s_current;
}

static const char *carrier_name(const td_link_carrier_t *carrier)
{
    return carrier && carrier->name ? carrier->name : "carrier";
}

td_link_state_t td_link_state(void)
{
    int64_t now = esp_timer_get_time() / 1000;
    portENTER_CRITICAL(&s_lock);
    td_link_state_t state = s_state;
    state.carrier = "none";
    if (s_carrier_count) {
        size_t slot = active_slot(now);
        state.online = slot_online(slot, now);
        state.last_rx_ms = s_carriers[slot].last_rx_ms;
        state.carrier = carrier_name(s_carriers[slot].carrier);
    }
    portEXIT_CRITICAL(&s_lock);
    return state;
}

bool td_link_carrier_online(const td_link_carrier_t *carrier)
{
    int64_t now = esp_timer_get_time() / 1000;
    bool online = false;
    portENTER_CRITICAL(&s_lock);
    for (size_t slot = 0; slot < s_carrier_count; slot++) {
        if (s_carriers[slot].carrier == carrier) {
            online = slot_online(slot, now);
        }
    }
    portEXIT_CRITICAL(&s_lock);
    return online;
}

bool td_link_usb_online(void)
{
    int64_t now = esp_timer_get_time() / 1000;
    portENTER_CRITICAL(&s_lock);
    const bool online = s_carrier_count && slot_online(0, now);
    portEXIT_CRITICAL(&s_lock);
    return online;
}

void td_link_carrier_offline(const td_link_carrier_t *carrier)
{
    portENTER_CRITICAL(&s_lock);
    for (size_t slot = 0; slot < s_carrier_count; slot++) {
        if (s_carriers[slot].carrier == carrier) {
            s_carriers[slot].online = false;
            s_carriers[slot].generation++;
        }
    }
    portEXIT_CRITICAL(&s_lock);
}

uint32_t td_link_carrier_generation(const td_link_carrier_t *carrier)
{
    uint32_t generation = 0;
    portENTER_CRITICAL(&s_lock);
    for (size_t slot = 0; slot < s_carrier_count; slot++) {
        if (s_carriers[slot].carrier == carrier) {
            generation = s_carriers[slot].generation;
        }
    }
    portEXIT_CRITICAL(&s_lock);
    return generation;
}

/* The slot in the top byte and the generation in the rest, never 0. Call under s_lock. */
static uint32_t origin_of(size_t slot, uint32_t generation)
{
    return (uint32_t)(slot + 1) << 24 | (generation & 0xffffffu);
}

static bool origin_live(uint32_t origin)
{
    const size_t slot = (origin >> 24) - 1;
    return origin && slot < s_carrier_count && origin_of(slot, s_carriers[slot].generation) == origin;
}

void td_link_transfer_claim(void)
{
    portENTER_CRITICAL(&s_lock);
    s_transfer = s_carrier_count ? origin_of(s_current, s_current_generation) : 0;
    portEXIT_CRITICAL(&s_lock);
}

bool td_link_transfer_ours(void)
{
    portENTER_CRITICAL(&s_lock);
    const bool ours = s_carrier_count && s_transfer == origin_of(s_current, s_current_generation);
    portEXIT_CRITICAL(&s_lock);
    return ours;
}

bool td_link_transfer_live(void)
{
    portENTER_CRITICAL(&s_lock);
    const bool live = origin_live(s_transfer);
    portEXIT_CRITICAL(&s_lock);
    return live;
}

bool td_link_current_network(void)
{
    portENTER_CRITICAL(&s_lock);
    const bool network = s_carrier_count && s_carriers[s_current].carrier->network;
    portEXIT_CRITICAL(&s_lock);
    return network;
}

const char *td_link_current_carrier(void)
{
    portENTER_CRITICAL(&s_lock);
    const char *name = carrier_name(s_carrier_count ? s_carriers[s_current].carrier : NULL);
    portEXIT_CRITICAL(&s_lock);
    return name;
}

esp_err_t td_link_messages_add(const td_link_message_t *messages, size_t count)
{
    if (!messages || !count) {
        return ESP_ERR_INVALID_ARG;
    }
    esp_err_t err = ESP_ERR_NO_MEM;
    portENTER_CRITICAL(&s_lock);
    for (size_t i = 0; i < TD_LINK_MESSAGE_TABLES && err != ESP_OK; i++) {
        if (!s_tables[i]) {
            s_table_counts[i] = count;
            s_tables[i] = messages;
            err = ESP_OK;
        }
    }
    portEXIT_CRITICAL(&s_lock);
    return err;
}

const td_link_message_t *td_link_extra_message(const char *name)
{
    const td_link_message_t *tables[TD_LINK_MESSAGE_TABLES];
    size_t counts[TD_LINK_MESSAGE_TABLES];
    portENTER_CRITICAL(&s_lock);
    for (size_t t = 0; t < TD_LINK_MESSAGE_TABLES; t++) {
        tables[t] = s_tables[t];
        counts[t] = s_table_counts[t];
    }
    portEXIT_CRITICAL(&s_lock);
    for (size_t t = 0; t < TD_LINK_MESSAGE_TABLES; t++) {
        for (size_t i = 0; tables[t] && i < counts[t]; i++) {
            if (!strcmp(tables[t][i].name, name)) {
                return &tables[t][i];
            }
        }
    }
    return NULL;
}

void td_link_hello(void)
{
    portENTER_CRITICAL(&s_lock);
    if (s_carrier_count) {
        s_carriers[s_current].online = true;
    }
    portEXIT_CRITICAL(&s_lock);
    if (s_presses) {
        xQueueReset(s_presses);
    }
}

void td_link_count_drop(void)
{
    portENTER_CRITICAL(&s_lock);
    s_state.dropped++;
    portEXIT_CRITICAL(&s_lock);
}

/* USB: SLIP over the USB-Serial/JTAG console, which the debug console shares. */
static bool usb_send(const td_link_carrier_t *carrier, const td_frame_t *frame)
{
    (void)carrier;
    static uint8_t wire[TD_FRAME_WIRE_MAX];
    size_t wire_len = td_frame_encode(frame, wire, sizeof wire);
    return wire_len && td_console_write((const char *)wire, wire_len);
}

static const td_link_carrier_t s_usb = { .send = usb_send, .name = "usb" };

/* USB goes first, into the empty table that td_link_start finds, and every other carrier after it. */
static esp_err_t add_carrier(const td_link_carrier_t *carrier)
{
    esp_err_t err = ESP_OK;
    portENTER_CRITICAL(&s_lock);
    if ((carrier == &s_usb) != (s_carrier_count == 0)) {
        err = ESP_ERR_INVALID_STATE;
    }
    for (size_t slot = 0; slot < s_carrier_count; slot++) {
        if (s_carriers[slot].carrier == carrier) {
            err = ESP_ERR_INVALID_STATE;
        }
    }
    if (err == ESP_OK && s_carrier_count == TD_LINK_CARRIERS_MAX) {
        err = ESP_ERR_NO_MEM;
    }
    if (err == ESP_OK) {
        s_carriers[s_carrier_count] = (carrier_slot_t){ .carrier = carrier };
        s_carrier_count++;
    }
    portEXIT_CRITICAL(&s_lock);
    return err;
}

esp_err_t td_link_carrier_add(const td_link_carrier_t *carrier)
{
    if (!carrier || !carrier->send) {
        return ESP_ERR_INVALID_ARG;
    }
    return add_carrier(carrier);
}

void td_link_carrier_receive(const uint8_t *body, size_t len, void *carrier)
{
    size_t slot = 0;
    portENTER_CRITICAL(&s_lock);
    while (slot < s_carrier_count && s_carriers[slot].carrier != carrier) {
        slot++;
    }
    const bool known = slot < s_carrier_count;
    portEXIT_CRITICAL(&s_lock);
    if (!known) {
        td_link_count_drop();
        return;
    }
    td_frame_t frame;
    if (td_frame_decode(body, len, &frame) != TD_FRAME_OK) {
        portENTER_CRITICAL(&s_lock);
        s_state.crc_errors++;
        portEXIT_CRITICAL(&s_lock);
        return;
    }
    int64_t now = esp_timer_get_time() / 1000;
    portENTER_CRITICAL(&s_lock);
    if (now - s_carriers[slot].last_rx_ms >= TD_LINK_IDLE_MS) {
        s_carriers[slot].online = false;
    }
    s_carriers[slot].last_rx_ms = now;
    s_state.frames_rx++;
    const uint32_t generation = s_carriers[slot].generation;
    portEXIT_CRITICAL(&s_lock);

    xSemaphoreTake(s_rx_mutex, portMAX_DELAY);
    if (xStreamBufferSpacesAvailable(s_frames) < len + TD_LINK_PACKET_HEADER) {
        xSemaphoreGive(s_rx_mutex);
        td_link_count_drop();
        return;
    }
    s_rx_packet[0] = (uint8_t)slot;
    s_rx_packet[1] = (uint8_t)(len & 0xff);
    s_rx_packet[2] = (uint8_t)(len >> 8);
    memcpy(s_rx_packet + 3, &generation, sizeof generation);
    memcpy(s_rx_packet + TD_LINK_PACKET_HEADER, body, len);
    size_t sent = xStreamBufferSend(s_frames, s_rx_packet, len + TD_LINK_PACKET_HEADER, 0);
    xSemaphoreGive(s_rx_mutex);
    configASSERT(sent == len + TD_LINK_PACKET_HEADER);
}

static bool read_bytes(uint8_t *buf, size_t len, TickType_t first_wait)
{
    size_t used = 0;
    while (used < len) {
        size_t got = xStreamBufferReceive(s_frames, buf + used, len - used, used == 0 ? first_wait : portMAX_DELAY);
        if (got == 0) {
            return false;
        }
        used += got;
    }
    return true;
}

/* The generation of the carrier connection the frame td_link_send hands a carrier belongs to. */
static uint32_t s_send_generation;

uint32_t td_link_send_generation(void)
{
    portENTER_CRITICAL(&s_lock);
    const uint32_t generation = s_send_generation;
    portEXIT_CRITICAL(&s_lock);
    return generation;
}

bool td_link_send(uint8_t type, uint8_t seq, uint8_t flags, const uint8_t *payload, size_t len)
{
    td_frame_t frame = { .type = type, .seq = seq, .flags = flags, .len = (uint16_t)len, .payload = payload };
    int64_t now = esp_timer_get_time() / 1000;
    const td_link_carrier_t *carrier = NULL;
    portENTER_CRITICAL(&s_lock);
    if (s_carrier_count) {
        /* A reply or an ACK answers the frame being handled, on the carrier it came on, and only while the connection
         * it came on lasts. */
        const size_t slot = flags & TD_FRAME_RESPONSE ? s_current : active_slot(now);
        if (!(flags & TD_FRAME_RESPONSE) || s_carriers[slot].generation == s_current_generation) {
            carrier = s_carriers[slot].carrier;
            s_send_generation = s_carriers[slot].generation;
        }
    }
    portEXIT_CRITICAL(&s_lock);
    if (!carrier || !carrier->send(carrier, &frame)) {
        td_link_count_drop();
        return false;
    }
    portENTER_CRITICAL(&s_lock);
    s_state.frames_tx++;
    portEXIT_CRITICAL(&s_lock);
    return true;
}

void td_link_fail(cJSON *reply, const char *error)
{
    cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
    cJSON_DeleteItemFromObjectCaseSensitive(reply, "error");
    cJSON_AddStringToObject(reply, "error", error);
}

static void handle_message(const td_frame_t *frame)
{
    static char payload[TD_FRAME_PAYLOAD_MAX + 1];
    memcpy(payload, frame->payload, frame->len);
    payload[frame->len] = '\0';
    const char *end = NULL;
    cJSON *request = cJSON_ParseWithLengthOpts(payload, frame->len + 1, &end, true);
    const char *raw_message = NULL;
    size_t message_len = 0;
    cJSON *recovered = NULL;
    if (td_v2_json_message(payload, frame->len, &raw_message, &message_len)) {
        recovered = cJSON_ParseWithLength(raw_message, message_len);
    }
    const char *message = cJSON_GetStringValue(recovered);
    bool strict = message && (!strncmp(message, "v2.", 3) || !strcmp(message, "hello"));
    cJSON *reply;
    if (strict && !td_v2_json_valid_live(payload, frame->len, !strcmp(message, "v2.live.set"))) {
        reply = cJSON_CreateObject();
        char echo[65];
        snprintf(echo, sizeof echo, "%.64s", message);
        cJSON_AddStringToObject(reply, "m", echo);
        cJSON_AddBoolToObject(reply, "ok", false);
        cJSON_AddStringToObject(reply, "reason", "args");
        cJSON_AddStringToObject(reply, "error", "Invalid request fields");
    } else if (!cJSON_IsObject(request) || end != payload + frame->len) {
        cJSON_Delete(recovered);
        cJSON_Delete(request);
        td_link_count_drop();
        return;
    } else {
        reply = td_link_dispatch(request);
    }
    cJSON_Delete(recovered);
    cJSON_Delete(request);
    bool printed = reply && cJSON_PrintPreallocated(reply, payload, sizeof payload, false);
    cJSON_Delete(reply);
    if (!printed) {
        td_link_count_drop();
        return;
    }
    td_link_send(TD_FRAME_MESSAGE, frame->seq, TD_FRAME_RESPONSE, (const uint8_t *)payload, strlen(payload));
}

/* Waits up to `wait` ticks for one frame, and handles it. */
static void handle_frame(TickType_t wait)
{
    static uint8_t body[TD_FRAME_BODY_MAX];
    uint8_t header[TD_LINK_PACKET_HEADER];
    if (!read_bytes(header, sizeof header, wait)) {
        return;
    }
    size_t len = header[1] | ((size_t)header[2] << 8);
    uint32_t generation;
    memcpy(&generation, header + 3, sizeof generation);
    read_bytes(body, len, portMAX_DELAY);
    td_frame_t frame;
    if (td_frame_decode(body, len, &frame) != TD_FRAME_OK) {
        td_link_count_drop();
        return;
    }
    portENTER_CRITICAL(&s_lock);
    /* A frame whose connection ended while it waited here is not handled: its reply would reach the next one. */
    const bool stale = s_carriers[header[0]].generation != generation;
    if (!stale) {
        s_current = header[0];
        s_current_generation = generation;
    }
    portEXIT_CRITICAL(&s_lock);
    const bool transfer_frame = frame.type == TD_FRAME_CHUNK || frame.type == TD_FRAME_ACK;
    if (stale || (transfer_frame && !td_link_transfer_ours())) {
        /* A transfer moves only on the carrier connection that opened it. */
        td_link_count_drop();
    } else if (frame.type == TD_FRAME_MESSAGE && frame.flags == 0) {
        handle_message(&frame);
    } else if (frame.type == TD_FRAME_CHUNK && frame.flags == 0) {
        td_link_files_chunk(frame.seq, frame.payload, frame.len);
    } else if (frame.type == TD_FRAME_ACK && frame.flags == TD_FRAME_RESPONSE) {
        td_link_files_ack(frame.payload, frame.len);
    } else {
        td_link_count_drop();
    }
}

static void queue_press(const char *page, const char *tile, const char *kind, const char *part, uint32_t generation)
{
    press_t press;
    if (!td_press_init(&press, page, tile, kind, part, td_link_v2_selected(), generation,
                       (uint32_t)(esp_timer_get_time() / 1000))) {
        return;
    }
    press.seq = __atomic_add_fetch(&s_press_seq, 1, __ATOMIC_RELAXED);
    xQueueSend(s_presses, &press, 0);
}

void td_link_press(const char *page, const char *tile, const char *kind)
{
    if (s_presses) {
        queue_press(page, tile, kind, NULL, ui_live_generation());
    }
}

void td_link_press_part(const char *page, const char *tile, const char *kind, const char *part, uint32_t generation)
{
    if (s_presses) {
        queue_press(page, tile, kind, part, generation);
    }
}

void td_link_session_reset(void)
{
    td_link_files_select();
    ui_live_select();
}

void td_link_input_cancel(void)
{
    portENTER_CRITICAL(&s_lock);
    td_input_cancel(&s_input);
    portEXIT_CRITICAL(&s_lock);
}

void td_link_input(const char *page, const char *tile, double value, bool end)
{
    if (!td_link_v2_selected()) { td_link_input_cancel(); return; }
    uint32_t now = (uint32_t)(esp_timer_get_time() / 1000);
    portENTER_CRITICAL(&s_lock);
    td_input_put(&s_input, page, tile, value, end, now);
    portEXIT_CRITICAL(&s_lock);
}

static void send_input(void)
{
    if (!td_link_v2_selected()) { td_link_input_cancel(); return; }
    td_input_event_t event;
    uint32_t now = (uint32_t)(esp_timer_get_time() / 1000);
    portENTER_CRITICAL(&s_lock);
    bool ready = td_input_take(&s_input, now, &event);
    portEXIT_CRITICAL(&s_lock);
    if (!ready) return;
    char json[256];
    int len = snprintf(json, sizeof json,
        "{\"m\":\"v2.input\",\"page\":\"%s\",\"tile\":\"%s\",\"stream\":\"value\",\"phase\":\"%s\",\"value\":%.4f,\"seq\":%u,\"t\":%u}",
        event.page, event.tile, event.end ? "end" : "move", event.value, (unsigned)event.seq, (unsigned)event.t);
    if (len > 0 && (size_t)len < sizeof json) td_link_send(TD_FRAME_MESSAGE, 0, TD_FRAME_EVENT, (const uint8_t *)json, (size_t)len);
}

static void send_page(void)
{
    static ui_page_report_t report;
    if (!td_link_v2_selected() || !ui_ready()) {
        return;
    }
    uint32_t layout = 0;
    const uint8_t page = ui_page_shown(&layout);
    const uint32_t session = td_link_v2_session();
    if (!ui_page_report_due(&report, session, layout, page)) {
        return;
    }
    char id[32];
    if (!ui_page_id(page, id, sizeof id)) {
        report.sent = false;
        return;
    }
    char json[64];
    int len = snprintf(json, sizeof json, "{\"m\":\"v2.page\",\"page\":\"%s\"}", id);
    if (len > 0 && (size_t)len < sizeof json) {
        td_link_send(TD_FRAME_MESSAGE, 0, TD_FRAME_EVENT, (const uint8_t *)json, (size_t)len);
    }
}

/* Events go out only while the app is connected. Otherwise they are thrown away, and their seq leaves a gap. */
static void send_events(void)
{
    static char json[KEY_SINK_LINK_EVENT_MAX];
    const bool online = td_link_state().online;
    press_t press;
    for (int turn = 0; turn < TD_LINK_EVENTS_PER_TURN && xQueueReceive(s_presses, &press, 0) == pdTRUE; turn++) {
        if (td_press_ready(&press, online, td_link_v2_selected(), ui_live_generation())) {
            const size_t len = td_press_json(&press, json, sizeof json);
            if (len > 0) {
                td_link_send(TD_FRAME_MESSAGE, 0, TD_FRAME_EVENT, (const uint8_t *)json, len);
            }
        }
    }
    if (!online) {
        key_sink_link_discard();
        return;
    }
    for (int turn = 0; turn < TD_LINK_EVENTS_PER_TURN; turn++) {
        size_t len = key_sink_link_next(json, sizeof json);
        if (len == 0) {
            break;
        }
        td_link_send(TD_FRAME_MESSAGE, 0, TD_FRAME_EVENT, (const uint8_t *)json, len);
    }
}

static void link_task(void *arg)
{
    (void)arg;
    for (;;) {
        handle_frame(pdMS_TO_TICKS(TD_LINK_POLL_MS));
        td_link_files_service((uint32_t)(esp_timer_get_time() / 1000));
        send_input();
        send_page();
        send_events();
    }
}

esp_err_t td_link_start(void)
{
    if (s_frames) {
        return ESP_ERR_INVALID_STATE;
    }
    s_frames = xStreamBufferCreate(TD_LINK_STREAM_BYTES, 1);
    s_presses = xQueueCreate(TD_LINK_PRESS_QUEUE, sizeof(press_t));
    s_rx_mutex = xSemaphoreCreateMutex();
    if (s_frames && s_presses && s_rx_mutex && add_carrier(&s_usb) == ESP_OK
        && xTaskCreate(link_task, "td_link", 8192, NULL, 3, NULL) == pdPASS) {
        td_console_set_frame_handler(td_link_carrier_receive, (void *)&s_usb);
        return ESP_OK;
    }
    portENTER_CRITICAL(&s_lock);
    s_carrier_count = 0;
    portEXIT_CRITICAL(&s_lock);
    if (s_frames) {
        vStreamBufferDelete(s_frames);
        s_frames = NULL;
    }
    if (s_presses) {
        vQueueDelete(s_presses);
        s_presses = NULL;
    }
    if (s_rx_mutex) {
        vSemaphoreDelete(s_rx_mutex);
        s_rx_mutex = NULL;
    }
    return ESP_ERR_NO_MEM;
}
