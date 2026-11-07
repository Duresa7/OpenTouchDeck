#pragma once

#include <stdbool.h>
#include <stddef.h>
#include "cJSON.h"
#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The `wifi.*` link messages set, read and forget the Device's Wi-Fi network (schema/protocol/link-v2.md). Only the
 * radio build has a Wi-Fi station, and it hands the link its handler. The normal build hands none, so every `wifi.*`
 * message answers `unsupported` there.
 *
 * The link checks the carrier first: Wi-Fi credentials move over USB only, so a `wifi.*` message that arrived on any
 * other carrier answers `usb-only` and reaches no handler. The handler gets the message name (`wifi.set`, ...), the
 * request and a reply that already holds `m` and `ok:true`. It fills the reply or refuses with td_link_wifi_fail. It
 * runs on the td_link task.
 */
typedef void (*td_link_wifi_handler_t)(const char *message, const cJSON *request, cJSON *reply);

/*
 * Sets the handler, once, before td_link_start. With a handler set, a v2 hello also advertises `caps.wifi` true.
 * Returns ESP_ERR_INVALID_ARG for NULL and ESP_ERR_INVALID_STATE when a handler is set already.
 */
esp_err_t td_link_wifi_register(td_link_wifi_handler_t handler);

/* Turns the reply into a refusal: `ok` false, the stable `reason` and the human `error`. */
void td_link_wifi_fail(cJSON *reply, const char *reason, const char *error);

/* True when the request has `m`, no field outside `allowed`, and no key twice. */
bool td_link_wifi_fields(const cJSON *request, const char *const *allowed, size_t count);

#ifdef __cplusplus
}
#endif
