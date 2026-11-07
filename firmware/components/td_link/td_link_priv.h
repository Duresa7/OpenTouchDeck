#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "td_link.h"

bool td_link_v2_selected(void);
uint32_t td_link_v2_session(void);

/* Everything below runs on the td_link task only. */

/* Sends one frame through the active carrier (td_link_carrier.h). Counts it in frames_tx, or in dropped when the
 * carrier did not take it. */
bool td_link_send(uint8_t type, uint8_t seq, uint8_t flags, const uint8_t *payload, size_t len);

/* True when the frame the link handles now arrived on USB. */
bool td_link_request_on_usb(void);

/* Counts a frame the link could not use in `dropped`. */
void td_link_count_drop(void);

/* Turns an ok reply into a refusal with this message. */
void td_link_fail(cJSON *reply, const char *error);

/* A v2 refusal: td_link_fail plus the stable `reason`. */
void td_link_v2_fail(cJSON *reply, const char *reason, const char *error);

/* True when the request has `m`, no other field outside `allowed`, and no key twice. */
bool td_link_v2_fields(const cJSON *request, const char *const *allowed, size_t count);

/* Chunk and ack frames, and the periodic work of a transfer: the chunks of a get that are due, and the abort of a
 * transfer that stopped moving. */
void td_link_files_chunk(uint8_t seq, const uint8_t *payload, size_t len);
void td_link_files_ack(const uint8_t *payload, size_t len);
void td_link_files_service(uint32_t now_ms);

/* A protocol selection, v1 or v2, discards an open v2 upload. */
void td_link_files_select(void);

/* A protocol selection starts a new session: discards the open upload, and clears the live values and the Live tile
 * touch, whose reset drops the queued input. Taps queued before it are dropped by their live-table generation. */
void td_link_session_reset(void);

/* Message handlers. Each fills the reply, or calls td_link_fail. */
void td_link_layout_put(const cJSON *request, cJSON *reply);
void td_link_layout_get(const cJSON *request, cJSON *reply);
void td_link_icon_put(const cJSON *request, cJSON *reply);
void td_link_icon_list(const cJSON *request, cJSON *reply);
void td_link_icon_rm(const cJSON *request, cJSON *reply);
void td_link_settings_get(const cJSON *request, cJSON *reply);
void td_link_settings_put(const cJSON *request, cJSON *reply);
void td_link_store_stat(const cJSON *request, cJSON *reply);
void td_link_xfer_end(const cJSON *request, cJSON *reply);
void td_link_update_begin(const cJSON *request, cJSON *reply);
void td_link_update_end(const cJSON *request, cJSON *reply);
void td_link_asset_stat(const cJSON *request, cJSON *reply);

/* v2.live.set: checks the request's fields and hands the value to the Layout on screen. */
void td_link_live_set(const cJSON *request, cJSON *reply);
void td_link_asset_list(const cJSON *request, cJSON *reply);
void td_link_asset_put(const cJSON *request, cJSON *reply);
void td_link_asset_end(const cJSON *request, cJSON *reply);
void td_link_asset_delete(const cJSON *request, cJSON *reply);
void td_link_photo_put(const cJSON *request, cJSON *reply);
void td_link_photo_end(const cJSON *request, cJSON *reply);
void td_link_idle_get(const cJSON *request, cJSON *reply);
void td_link_idle_set(const cJSON *request, cJSON *reply);
void td_link_clock_set(const cJSON *request, cJSON *reply);

/* The interval between pictures in the idle settings in force. Any task. */
uint32_t td_link_idle_interval_s(void);

/* The `layout state` fields, from the layout on screen. */
void td_link_layout_fields(cJSON *reply);
