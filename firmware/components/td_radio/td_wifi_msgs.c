#include "td_wifi.h"
#include <stdio.h>
#include <string.h>
#include "td_console.h"

static void wipe(char *s)
{
    volatile char *p = s;
    while (*p) {
        *p++ = '\0';
    }
}

void td_wifi_state_fields(cJSON *reply, const td_wifi_state_t *state)
{
    const bool up = state->status == TD_WIFI_CONNECTED;
    const char *reason = td_wifi_reason_name(state->reason);
    cJSON_AddBoolToObject(reply, "configured", state->configured);
    cJSON_AddStringToObject(reply, "ssid", state->ssid);
    cJSON_AddStringToObject(reply, "state", td_wifi_status_name(state->status));
    cJSON_AddItemToObject(reply, "reason", reason ? cJSON_CreateString(reason) : cJSON_CreateNull());
    cJSON_AddItemToObject(reply, "ip", up && state->ip[0] ? cJSON_CreateString(state->ip) : cJSON_CreateNull());
    cJSON_AddItemToObject(reply, "rssi", up && state->has_rssi ? cJSON_CreateNumber(state->rssi) : cJSON_CreateNull());
}

/* Turns the reply into a refusal: `ok` false, the stable `reason` and the human `error`. */
static void fail(cJSON *reply, const char *reason, const char *error)
{
    cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
    cJSON_DeleteItemFromObjectCaseSensitive(reply, "error");
    cJSON_AddStringToObject(reply, "reason", reason);
    cJSON_AddStringToObject(reply, "error", error);
}

/* True when the request has `m`, no field outside `allowed`, and no key twice. */
static bool fields(const cJSON *request, const char *const *allowed, size_t count)
{
    const cJSON *a;
    cJSON_ArrayForEach(a, request) {
        bool known = !strcmp(a->string, "m");
        for (size_t i = 0; i < count && !known; i++) {
            known = !strcmp(a->string, allowed[i]);
        }
        if (!known) {
            return false;
        }
        for (const cJSON *b = a->next; b; b = b->next) {
            if (!strcmp(a->string, b->string)) {
                return false;
            }
        }
    }
    return true;
}

/* Refuses with the stable reason for a td_wifi_set or td_wifi_forget error. */
static void refuse(cJSON *reply, esp_err_t err)
{
    if (err == ESP_ERR_INVALID_ARG) {
        fail(reply, "args", "Invalid network name or password");
    } else {
        fail(reply, "storage", "The network could not be stored");
    }
}

void td_wifi_link_caps(cJSON *caps)
{
    cJSON_AddBoolToObject(caps, "wifi", true);
}

void td_wifi_link_message(const cJSON *request, cJSON *reply)
{
    static const char *const set_fields[] = { "ssid", "password" };
    const char *message = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "m"));
    if (!message) {
        fail(reply, "args", "Invalid request");
        return;
    }
    if (!strcmp(message, "wifi.state") || !strcmp(message, "wifi.forget")) {
        if (!fields(request, NULL, 0)) {
            fail(reply, "args", "Invalid request fields");
            return;
        }
        const esp_err_t err = !strcmp(message, "wifi.forget") ? td_wifi_forget() : ESP_OK;
        if (err != ESP_OK) {
            refuse(reply, err);
            return;
        }
    } else if (!strcmp(message, "wifi.set")) {
        const cJSON *ssid = cJSON_GetObjectItemCaseSensitive(request, "ssid");
        const cJSON *password = cJSON_GetObjectItemCaseSensitive(request, "password");
        esp_err_t err = ESP_ERR_INVALID_ARG;
        if (fields(request, set_fields, 2) && cJSON_IsString(ssid) && cJSON_IsString(password)) {
            err = td_wifi_set(ssid->valuestring, password->valuestring);
        }
        /* The request is freed after the reply is built. Its password goes first. */
        if (cJSON_IsString(password)) {
            wipe(password->valuestring);
        }
        if (err != ESP_OK) {
            refuse(reply, err);
            return;
        }
    } else {
        fail(reply, "unsupported", "Unsupported request");
        return;
    }
    const td_wifi_state_t state = td_wifi_state();
    td_wifi_state_fields(reply, &state);
}

/* `wifi state`, `wifi set <ssid> <password>` and `wifi forget`. The console masks the password as it is typed
 * (td_console_mask_after), and the reply carries the same fields as the link's. */
static int cmd_wifi(int argc, char **argv)
{
    cJSON *reply = cJSON_CreateObject();
    if (!reply) {
        char buf[TD_REPLY_MAX];
        td_reply_error(buf, sizeof buf, "wifi", "out of memory");
        td_console_write_line(buf);
        return 0;
    }
    cJSON_AddStringToObject(reply, "cmd", "wifi");
    cJSON_AddBoolToObject(reply, "ok", true);
    esp_err_t err = ESP_OK;
    bool usage = false;
    if (argc == 4 && !strcmp(argv[1], "set")) {
        err = td_wifi_set(argv[2], argv[3]);
        wipe(argv[3]);
    } else if (argc == 2 && !strcmp(argv[1], "forget")) {
        err = td_wifi_forget();
    } else if (!(argc == 2 && !strcmp(argv[1], "state"))) {
        usage = true;
        for (int i = 3; i < argc; i++) {
            wipe(argv[i]);
        }
    }
    if (usage) {
        fail(reply, "args", "usage: wifi state | set <ssid> <password> | forget");
    } else if (err != ESP_OK) {
        refuse(reply, err);
    } else {
        const td_wifi_state_t state = td_wifi_state();
        td_wifi_state_fields(reply, &state);
    }
    char line[TD_REPLY_MAX];
    if (!cJSON_PrintPreallocated(reply, line + 4, sizeof line - 4, false)) {
        td_reply_error(line, sizeof line, "wifi", "reply too long");
    } else {
        memcpy(line, "@td ", 4);
    }
    cJSON_Delete(reply);
    td_console_write_line(line);
    return 0;
}

const esp_console_cmd_t *td_wifi_console_commands(size_t *count)
{
    static const esp_console_cmd_t cmds[] = {
        { .command = "wifi", .help = "report, set or forget the Wi-Fi network; the password is masked as you type",
          .hint = "state | set <ssid> <password> | forget", .func = cmd_wifi },
    };
    *count = sizeof cmds / sizeof cmds[0];
    return cmds;
}
