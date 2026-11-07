#pragma once

#include <stdbool.h>
#include <stdint.h>
#include "cJSON.h"
#include "td_console.h"

typedef struct {
    bool online;
    uint32_t frames_rx;
    uint32_t frames_tx;
    uint32_t crc_errors;
    uint32_t dropped;
    int64_t last_rx_ms;
    /* The active carrier's name (td_link_carrier.h). */
    const char *carrier;
} td_link_state_t;

typedef struct {
    int image[2];
    uint32_t min_delay_ms;
    uint32_t page_tiles;
    uint32_t page_pixels;
} td_link_animation_caps_t;

cJSON *td_link_capabilities(void);

#define TD_LINK_ID_MAX 16

esp_err_t td_link_start(void);

/* Applies the photo frame's stored idle settings to the power gate and the idle face. Call it once after power_start;
 * v2.idle.set applies later changes. */
void td_link_idle_boot(void);

/* Queues a `press` event for a tap on an app or widget tile, from any task and without blocking. It is dropped when
 * the queue is full or the app is not connected, and when the live-table generation has moved on by the time it would
 * go out: td_link_press takes the generation as it is now, and td_link_press_part the one its part was named in. */
void td_link_press(const char *page, const char *tile, const char *kind);
void td_link_press_part(const char *page, const char *tile, const char *kind, const char *part, uint32_t generation);
/* Called by the bar renderer after drag slop. Cancel on Layout changes. */
void td_link_input(const char *page, const char *tile, double value, bool end);
void td_link_input_cancel(void);
td_link_state_t td_link_state(void);
void td_link_hello(void);
cJSON *td_link_dispatch(const cJSON *request);

/* A message another component handles. A usb_only message is refused on a network carrier. */
typedef struct {
    const char *name;
    void (*handler)(const cJSON *request, cJSON *reply);
    bool usb_only;
} td_link_message_t;

#define TD_LINK_MESSAGE_TABLES 2

/* Adds a table of messages, which must stay valid for good, before or after td_link_start, from a task. The link
 * sees the whole table or none of it. ESP_ERR_INVALID_ARG for an empty table, ESP_ERR_NO_MEM when
 * TD_LINK_MESSAGE_TABLES are in use. */
esp_err_t td_link_messages_add(const td_link_message_t *messages, size_t count);
const esp_console_cmd_t *td_link_console_commands(size_t *count);
