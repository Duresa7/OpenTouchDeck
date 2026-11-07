/*
 * `ble state|on|off|pair|close|forget [address]` on the USB console. Each runs the matching `ble.*` link message, so
 * the console and the app see the same rules and the same answers.
 */
#include <string.h>
#include "cJSON.h"
#include "td_ble.h"
#include "td_ble_service.h"
#include "td_console.h"

#define USAGE "usage: ble state|on|off|pair|close|forget [aa:bb:cc:dd:ee:ff]"

static cJSON *request_for(int argc, char **argv)
{
    if (argc < 2 || argc > 3) {
        return NULL;
    }
    const char *op = argv[1];
    cJSON *request = cJSON_CreateObject();
    if (!request) {
        return NULL;
    }
    bool ok = argc == 2;
    if (!strcmp(op, "state")) {
        cJSON_AddStringToObject(request, "m", "ble.state");
    } else if (!strcmp(op, "on") || !strcmp(op, "off")) {
        cJSON_AddStringToObject(request, "m", "ble.set");
        cJSON_AddBoolToObject(request, "enabled", !strcmp(op, "on"));
    } else if (!strcmp(op, "pair") || !strcmp(op, "close")) {
        cJSON_AddStringToObject(request, "m", "ble.pair");
        cJSON_AddBoolToObject(request, "open", !strcmp(op, "pair"));
    } else if (!strcmp(op, "forget")) {
        cJSON_AddStringToObject(request, "m", "ble.forget");
        if (argc == 3) {
            cJSON_AddStringToObject(request, "address", argv[2]);
        }
        ok = true;
    } else {
        ok = false;
    }
    if (!ok) {
        cJSON_Delete(request);
        return NULL;
    }
    return request;
}

static const char *string_of(const cJSON *reply, const char *key)
{
    return cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, key));
}

static int cmd_ble(int argc, char **argv)
{
    char buf[TD_REPLY_MAX];
    cJSON *request = request_for(argc, argv);
    cJSON *reply = cJSON_CreateObject();
    if (!request || !reply || !cJSON_AddBoolToObject(reply, "ok", true)) {
        cJSON_Delete(request);
        cJSON_Delete(reply);
        td_reply_error(buf, sizeof buf, "ble", request ? "out of memory" : USAGE);
        td_console_write_line(buf);
        return 0;
    }
    td_ble_message(request, reply);
    const bool ok = cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "ok"));
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "ble", ok);
    td_reply_str(&r, "op", argv[1]);
    if (!ok) {
        td_reply_str(&r, "reason", string_of(reply, "reason"));
        td_reply_str(&r, "error", string_of(reply, "error"));
    }
    const cJSON *forgotten = cJSON_GetObjectItemCaseSensitive(reply, "forgotten");
    if (cJSON_IsNumber(forgotten)) {
        td_reply_int(&r, "forgotten", (long long)forgotten->valuedouble);
    }
    const cJSON *enabled = cJSON_GetObjectItemCaseSensitive(reply, "enabled");
    if (enabled) {
        td_reply_bool(&r, "enabled", cJSON_IsTrue(enabled));
        td_reply_str(&r, "state", string_of(reply, "state"));
        td_reply_bool(&r, "pairing", cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "pairing")));
        td_reply_int(&r, "pairing_ms",
                     (long long)cJSON_GetNumberValue(cJSON_GetObjectItemCaseSensitive(reply, "pairing_ms")));
        td_reply_str(&r, "advertising", string_of(reply, "advertising"));
        const char *addresses[TD_BLE_BONDS_MAX];
        const char *connected[TD_BLE_BONDS_MAX];
        size_t count = 0;
        size_t live = 0;
        const cJSON *bond;
        cJSON_ArrayForEach(bond, cJSON_GetObjectItemCaseSensitive(reply, "bonds")) {
            const char *address = string_of(bond, "address");
            if (address && count < TD_BLE_BONDS_MAX) {
                addresses[count++] = address;
                if (cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(bond, "connected"))) {
                    connected[live++] = address;
                }
            }
        }
        td_reply_strs(&r, "bonds", addresses, count);
        td_reply_strs(&r, "connected", connected, live);
    }
    td_reply_end(&r);
    td_console_write_line(r.buf);
    cJSON_Delete(request);
    cJSON_Delete(reply);
    return 0;
}

const esp_console_cmd_t *td_ble_console_commands(size_t *count)
{
    static const esp_console_cmd_t cmds[] = {
        { .command = "ble",
          .help = "the Bluetooth keyboard: turn it on or off (stored), open or close the pairing window, forget hosts",
          .hint = "state|on|off|pair|close|forget [address]",
          .func = cmd_ble },
    };
    *count = sizeof cmds / sizeof cmds[0];
    return cmds;
}
