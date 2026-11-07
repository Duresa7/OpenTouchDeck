#include "td_link.h"
#include "td_link_priv.h"
#include "ui.h"
#include <math.h>
#include <stdio.h>
#include <string.h>
#include "esp_app_desc.h"
#include "esp_ota_ops.h"
#include "esp_timer.h"
#include "td_recovery.h"
#include "td_device.h"
#include "td_asset.h"
#include "esp_mac.h"

static const td_link_extension_t *s_extensions[TD_LINK_EXTENSIONS_MAX];
static size_t s_extension_count;

esp_err_t td_link_extend(const td_link_extension_t *extension)
{
    const size_t len = extension && extension->prefix ? strlen(extension->prefix) : 0;
    if (!extension || !extension->handle || len == 0 || extension->prefix[len - 1] != '.' ||
        !strncmp(extension->prefix, "v2.", 3)) {
        return ESP_ERR_INVALID_ARG;
    }
    for (size_t i = 0; i < s_extension_count; i++) {
        if (!strcmp(s_extensions[i]->prefix, extension->prefix)) {
            return ESP_ERR_INVALID_STATE;
        }
    }
    if (s_extension_count == TD_LINK_EXTENSIONS_MAX) {
        return ESP_ERR_NO_MEM;
    }
    s_extensions[s_extension_count++] = extension;
    return ESP_OK;
}

cJSON *td_link_capabilities(void)
{
    cJSON *caps = cJSON_Parse("{\"screen\":[1024,600],\"grid\":[8,8],\"pages\":8,\"tiles\":64,\"layout_bytes\":32768,\"layout_schemas\":[1,2],\"tile_kinds\":[\"static\",\"graph\",\"photo-frame\",\"clock\",\"bar\",\"now-playing\"],\"asset_bytes\":32768,\"formats\":[\"tdi1\",\"jpeg\"],\"image\":[256,256],\"gif_frames\":0,\"gif_pixels\":0,\"photo_bytes\":131072,\"live_hz\":0,\"live_total_hz\":0,\"series\":0,\"text_bytes\":0,\"streams\":0}");
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "live_hz"), UI_LIVE_HZ);
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "live_total_hz"), UI_LIVE_TOTAL_HZ);
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "series"), UI_LIVE_SERIES_MAX);
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "streams"), UI_LIVE_TILE_STREAMS);
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "text_bytes"), UI_LIVE_TEXT_MAX);
    cJSON_AddNumberToObject(caps, "live_images", UI_LIVE_IMAGES_MAX);
#if CONFIG_TD_GIF_CAPS
    const td_link_animation_caps_t anim = { .image = {128, 128}, .min_delay_ms = 100, .page_tiles = 4, .page_pixels = 65536 };
    cJSON *object = cJSON_AddObjectToObject(caps, "anim");
    cJSON_AddItemToObject(object, "image", cJSON_CreateIntArray(anim.image, 2));
    cJSON_AddNumberToObject(object, "min_delay_ms", anim.min_delay_ms);
    cJSON_AddNumberToObject(object, "page_tiles", anim.page_tiles);
    cJSON_AddNumberToObject(object, "page_pixels", anim.page_pixels);
    const char *formats[] = {"tdi1", "gif", "jpeg"};
    const char *kinds[] = {"static", "graph", "gif", "photo-frame", "clock", "bar", "now-playing"};
    cJSON_ReplaceItemInObjectCaseSensitive(caps, "formats", cJSON_CreateStringArray(formats, 3));
    cJSON_ReplaceItemInObjectCaseSensitive(caps, "tile_kinds", cJSON_CreateStringArray(kinds, 7));
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "asset_bytes"), 262144);
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "gif_frames"), 60);
    cJSON_SetNumberValue(cJSON_GetObjectItemCaseSensitive(caps, "gif_pixels"), 983040);
#endif
    for (size_t i = 0; i < s_extension_count; i++) {
        if (caps && s_extensions[i]->caps) {
            s_extensions[i]->caps(caps);
        }
    }
    return caps;
}

static bool s_v2;
static uint32_t s_v2_session;
bool td_link_v2_selected(void) { return s_v2 && td_link_state().online; }
uint32_t td_link_v2_session(void) { return __atomic_load_n(&s_v2_session, __ATOMIC_RELAXED); }

void td_link_v2_fail(cJSON *reply, const char *reason, const char *error)
{
    td_link_fail(reply, error);
    cJSON_AddStringToObject(reply, "reason", reason);
}

#define v2_fail td_link_v2_fail

bool td_link_v2_fields(const cJSON *request, const char *const *allowed, size_t count)
{
    const cJSON *a;
    cJSON_ArrayForEach(a, request) {
        bool known = !strcmp(a->string, "m");
        for (size_t i = 0; i < count && !known; i++) {
            known = !strcmp(a->string, allowed[i]);
        }
        if (!known) return false;
        for (const cJSON *b = a->next; b; b = b->next) {
            if (!strcmp(a->string, b->string)) return false;
        }
    }
    return true;
}

static bool fields(const cJSON *request, const char *extra)
{
    return td_link_v2_fields(request, &extra, extra ? 1 : 0);
}


static bool link_online(void)
{
    return td_link_state().online;
}

static void hello(const cJSON *request, cJSON *reply)
{
    const cJSON *selector = cJSON_GetObjectItemCaseSensitive(request, "proto");
    if (!fields(request, "proto") || (selector && (!cJSON_IsNumber(selector) || floor(selector->valuedouble) != selector->valuedouble))) {
        v2_fail(reply, "args", "Invalid protocol selector");
        return;
    }
    if (selector && selector->valuedouble != 1 && selector->valuedouble != 2) {
        v2_fail(reply, "unsupported", "Unsupported protocol");
        return;
    }
    bool v2 = selector && selector->valueint == 2;
    char name[TD_DEVICE_NAME_MAX + 1];
    if (v2 && td_device_name_get(name) != ESP_OK) {
        v2_fail(reply, "storage", "Device name unavailable");
        return;
    }
    s_v2 = v2;
    if (v2) {
        __atomic_add_fetch(&s_v2_session, 1, __ATOMIC_RELAXED);
    }
    td_link_session_reset();
    const esp_app_desc_t *app = esp_app_get_description();
    const esp_partition_t *running = esp_ota_get_running_partition();
    char sha[2 * sizeof app->app_elf_sha256 + 1];
    for (size_t i = 0; i < sizeof app->app_elf_sha256; i++) {
        snprintf(sha + 2 * i, 3, "%02x", app->app_elf_sha256[i]);
    }
    td_link_hello();
    ui_set_link_online(link_online);
    cJSON_AddStringToObject(reply, "product", "touchdeck");
    cJSON_AddNumberToObject(reply, "proto", v2 ? 2 : 1);
    const int protocols[] = {1, 2};
    cJSON_AddItemToObject(reply, "protocols", cJSON_CreateIntArray(protocols, 2));
    cJSON_AddStringToObject(reply, "elf_sha256", sha);
    cJSON_AddStringToObject(reply, "version", app->version);
    cJSON_AddStringToObject(reply, "running", running ? running->label : "unknown");
    const char *sinks[] = { "log", "link" };
    cJSON_AddItemToObject(reply, "sinks", cJSON_CreateStringArray(sinks, 2));
    cJSON_AddBoolToObject(reply, "recovery", td_recovery_mode());
    ui_layout_info_t layout = { 0 };
    if (ui_ready()) {
        ui_layout_info(&layout);
    }
    cJSON_AddStringToObject(reply, "layout", layout.from_storage ? "storage" : "builtin");
    if (v2) {
        uint8_t mac[6];
        esp_efuse_mac_get_default(mac);
        char id[13];
        snprintf(id, sizeof id, "%02x%02x%02x%02x%02x%02x", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
        cJSON_AddStringToObject(reply, "id", id);
        cJSON_AddStringToObject(reply, "name", name);
        cJSON *caps = td_link_capabilities();
        if (strcmp(ui_layout_slot(), "portrait") == 0) {
            const int screen[] = {600, 1024};
            cJSON_ReplaceItemInObjectCaseSensitive(caps, "screen", cJSON_CreateIntArray(screen, 2));
        }
        td_asset_space_t space;
        td_asset_space(&space);
        cJSON_AddNumberToObject(caps, "free_bytes", space.free_bytes);
        cJSON_AddItemToObject(reply, "caps", caps);
    }
}

static void ping(const cJSON *request, cJSON *reply)
{
    const cJSON *n = cJSON_GetObjectItemCaseSensitive(request, "n");
    cJSON_AddNumberToObject(reply, "t", esp_timer_get_time() / 1000);
    if (cJSON_IsNumber(n) || cJSON_IsString(n) || cJSON_IsBool(n) || cJSON_IsNull(n)) {
        cJSON_AddItemToObject(reply, "n", cJSON_Duplicate(n, false));
    }
}

static void state(const cJSON *request, cJSON *reply)
{
    (void)request;
    td_link_state_t value = td_link_state();
    cJSON_AddBoolToObject(reply, "online", value.online);
    cJSON_AddNumberToObject(reply, "frames_rx", value.frames_rx);
    cJSON_AddNumberToObject(reply, "frames_tx", value.frames_tx);
    cJSON_AddNumberToObject(reply, "crc_errors", value.crc_errors);
    cJSON_AddNumberToObject(reply, "dropped", value.dropped);
    cJSON_AddNumberToObject(reply, "last_rx_ms", (double)value.last_rx_ms);
}

/* A missing optional field takes its default. A field of the wrong type is refused. */
static bool opt_string(const cJSON *request, const char *key, const char **out)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(request, key);
    *out = "";
    if (item == NULL) {
        return true;
    }
    if (!cJSON_IsString(item)) {
        return false;
    }
    *out = item->valuestring;
    return true;
}

static bool opt_bar(const cJSON *item, float *out)
{
    *out = -1.0f;
    if (item == NULL || cJSON_IsNull(item)) {
        return true;
    }
    if (!cJSON_IsNumber(item)) {
        return false;
    }
    *out = (float)item->valuedouble;
    return true;
}

static void widget_set(const cJSON *request, cJSON *reply)
{
    const cJSON *tile = cJSON_GetObjectItemCaseSensitive(request, "tile");
    const cJSON *stale = cJSON_GetObjectItemCaseSensitive(request, "stale");
    const char *value = NULL;
    const char *caption = NULL;
    float bar = -1.0f;
    if (!cJSON_IsString(tile)) {
        td_link_fail(reply, "tile must be a string");
        return;
    }
    if (!opt_string(request, "value", &value) || !opt_string(request, "caption", &caption)) {
        td_link_fail(reply, "value and caption must be strings");
        return;
    }
    if (!opt_bar(cJSON_GetObjectItemCaseSensitive(request, "bar"), &bar)) {
        td_link_fail(reply, "bar must be a number");
        return;
    }
    if (stale != NULL && !cJSON_IsBool(stale)) {
        td_link_fail(reply, "stale must be a boolean");
        return;
    }
    ui_widget_state_t stored;
    const char *error = ui_widget_update(tile->valuestring, value, caption, bar, cJSON_IsTrue(stale), &stored);
    if (error != NULL) {
        td_link_fail(reply, error);
        return;
    }
    cJSON_AddStringToObject(reply, "tile", tile->valuestring);
    cJSON_AddStringToObject(reply, "value", stored.value);
    cJSON_AddStringToObject(reply, "caption", stored.caption);
    cJSON_AddNumberToObject(reply, "bar", stored.bar < 0.0f ? -1.0 : round(stored.bar * 1000.0) / 1000.0);
    cJSON_AddBoolToObject(reply, "stale", stored.stale);
}

static void dashboard_set(const cJSON *request, cJSON *reply)
{
    static ui_dash_row_t rows[UI_DASH_ROWS_MAX];
    const cJSON *page = cJSON_GetObjectItemCaseSensitive(request, "page");
    const cJSON *list = cJSON_GetObjectItemCaseSensitive(request, "rows");
    if (!cJSON_IsString(page)) {
        td_link_fail(reply, "page must be a string");
        return;
    }
    if (!cJSON_IsArray(list)) {
        td_link_fail(reply, "rows must be an array");
        return;
    }
    const int count = cJSON_GetArraySize(list);
    if (count > UI_DASH_ROWS_MAX) {
        td_link_fail(reply, "too many rows");
        return;
    }
    int i = 0;
    const cJSON *row = NULL;
    cJSON_ArrayForEach(row, list)
    {
        const char *label = NULL;
        const char *value = NULL;
        if (!cJSON_IsObject(row) || !opt_string(row, "label", &label) || !opt_string(row, "value", &value) ||
            !opt_bar(cJSON_GetObjectItemCaseSensitive(row, "bar"), &rows[i].bar)) {
            td_link_fail(reply, "each row is an object with string label and value and a number bar");
            return;
        }
        ui_widget_copy_text(rows[i].label, sizeof rows[i].label, label);
        ui_widget_copy_text(rows[i].value, sizeof rows[i].value, value);
        i++;
    }
    const char *error = ui_dashboard_update(page->valuestring, rows, (size_t)count);
    if (error != NULL) {
        td_link_fail(reply, error);
        return;
    }
    cJSON_AddStringToObject(reply, "page", page->valuestring);
    cJSON_AddNumberToObject(reply, "rows", count);
}

static const struct {
    const char *name;
    void (*handler)(const cJSON *request, cJSON *reply);
} s_messages[] = {
    { "hello", hello },
    { "ping", ping },
    { "link.state", state },
    { "layout.put", td_link_layout_put },
    { "layout.get", td_link_layout_get },
    { "icon.put", td_link_icon_put },
    { "icon.list", td_link_icon_list },
    { "icon.rm", td_link_icon_rm },
    { "settings.get", td_link_settings_get },
    { "settings.put", td_link_settings_put },
    { "store.stat", td_link_store_stat },
    { "xfer.end", td_link_xfer_end },
    { "update.begin", td_link_update_begin },
    { "update.end", td_link_update_end },

    { "widget.set", widget_set },
    { "dashboard.set", dashboard_set },
};

static const struct {
    const char *name;
    void (*handler)(const cJSON *request, cJSON *reply);
} s_v2_messages[] = {
    { "v2.asset.stat", td_link_asset_stat },
    { "v2.asset.list", td_link_asset_list },
    { "v2.asset.put", td_link_asset_put },
    { "v2.asset.end", td_link_asset_end },
    { "v2.asset.delete", td_link_asset_delete },
    { "v2.live.set", td_link_live_set },
    { "v2.photo.put", td_link_photo_put },
    { "v2.photo.end", td_link_photo_end },
    { "v2.idle.get", td_link_idle_get },
    { "v2.idle.set", td_link_idle_set },
    { "v2.clock.set", td_link_clock_set },
};

#define ECHO_CHARS_MAX 64

static void echo_name(const char *name, char out[4 * ECHO_CHARS_MAX + 1])
{
    size_t len = 0;
    size_t chars = 0;
    while (name[len] && len < 4 * ECHO_CHARS_MAX) {
        if (((unsigned char)name[len] & 0xc0) != 0x80 && chars++ == ECHO_CHARS_MAX) {
            break;
        }
        len++;
    }
    memcpy(out, name, len);
    out[len] = '\0';
}

cJSON *td_link_dispatch(const cJSON *request)
{
    const cJSON *message = cJSON_GetObjectItemCaseSensitive(request, "m");
    if (!cJSON_IsString(message)) {
        return NULL;
    }
    cJSON *reply = cJSON_CreateObject();
    if (!reply) {
        return NULL;
    }
    char name[4 * ECHO_CHARS_MAX + 1];
    echo_name(message->valuestring, name);
    cJSON_AddStringToObject(reply, "m", name);
    if (!strncmp(message->valuestring, "v2.", 3)) {
        cJSON_AddBoolToObject(reply, "ok", true);
        if (!s_v2 || !td_link_state().online) {
            s_v2 = false;
            v2_fail(reply, "not-negotiated", "Select protocol 2 first");
        } else if (!strcmp(name, "v2.name.get") || !strcmp(name, "v2.name.set")) {
            bool set = !strcmp(name, "v2.name.set");
            const char *value = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "name"));
            char stored[TD_DEVICE_NAME_MAX + 1];
            if (!fields(request, set ? "name" : NULL) || (set && !td_device_name_valid(value))) {
                v2_fail(reply, "args", "Invalid name");
            } else if ((set && td_device_name_set(value) != ESP_OK) || td_device_name_get(stored) != ESP_OK) {
                v2_fail(reply, "storage", "Device name unavailable");
            } else {
                cJSON_AddStringToObject(reply, "name", stored);
            }
        } else {
            bool found = false;
            for (size_t i = 0; i < sizeof s_v2_messages / sizeof s_v2_messages[0] && !found; i++) {
                found = !strcmp(name, s_v2_messages[i].name);
                if (found) {
                    s_v2_messages[i].handler(request, reply);
                }
            }
            if (!found) {
                v2_fail(reply, "unsupported", "Unsupported request");
            }
        }
        return reply;
    }
    for (size_t i = 0; i < sizeof s_messages / sizeof s_messages[0]; i++) {
        if (strcmp(message->valuestring, s_messages[i].name) == 0) {
            cJSON_AddBoolToObject(reply, "ok", true);
            s_messages[i].handler(request, reply);
            return reply;
        }
    }
    for (size_t i = 0; i < s_extension_count; i++) {
        if (!strncmp(message->valuestring, s_extensions[i]->prefix, strlen(s_extensions[i]->prefix))) {
            cJSON_AddBoolToObject(reply, "ok", true);
            if (td_link_request_on_usb()) {
                s_extensions[i]->handle(request, reply);
            } else {
                v2_fail(reply, "usb-only", "Only over USB");
            }
            return reply;
        }
    }
    cJSON_AddBoolToObject(reply, "ok", false);
    cJSON_AddStringToObject(reply, "error", "unknown message");
    return reply;
}
