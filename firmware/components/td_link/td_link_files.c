#include "td_link_priv.h"
#include <math.h>
#include <stdio.h>
#include <string.h>
#include "esp_heap_caps.h"
#include "esp_timer.h"
#include "power.h"
#include "td_frame.h"
#include "td_asset.h"
#include "td_gif.h"
#include "td_idle.h"
#include "td_layout.h"
#include "td_store.h"
#include "td_update.h"
#include "td_update_state.h"
#include "td_xfer.h"
#include "ui.h"
#include "ui_gif_player.h"

#define LAYOUT_DIR "/storage/layouts"
#define ICON_DIR "/storage/icons"
#define ICON_ID_LEN 16
#define ICON_FILE_BYTES 12300
#define ICON_PAGE 32
#define GET_IDLE_MS 5000

typedef enum {
    KIND_NONE,
    KIND_LAYOUT_PUT,
    KIND_ICON_PUT,
    KIND_GET,
    KIND_UPDATE,
    KIND_ASSET_PUT,
    KIND_PHOTO_PUT,
} kind_t;

/* One transfer at a time. A put holds the whole file in PSRAM and writes it with td_store_write_atomic at xfer.end,
 * so a cut transfer leaves nothing on the volume. A get reads the file once and sends it from memory. A firmware
 * update holds no buffer: td_update writes each chunk into the other OTA slot as it arrives. A v2 Asset put holds no
 * buffer either: td_asset streams each chunk into a temporary file and hashes it, and v2.asset.end commits it. */
static struct {
    kind_t kind;
    td_xfer_t x;
    uint8_t *buf;
    size_t fill;
    char path[TD_STORE_PATH_MAX];
    char orientation[10];
    char icon_id[ICON_ID_LEN + 1];
    uint32_t last_ms;
    uint8_t last_id;
    uint8_t tx_seq;
    const char *asset_failure;
    char asset_id[TD_ASSET_ID_LEN + 1];
    uint8_t photo_sha256[TD_SHA256_BYTES];
    uint32_t photo_entry;
} s;

/* The outcome of the last v2 Asset transfer that closed, which v2.asset.end repeats for its xfer until the next bulk
 * open: the committed id and size, or the refusal reason. */
static struct {
    uint8_t xfer;
    const char *reason;
    char id[TD_ASSET_ID_LEN + 1];
    uint32_t bytes;
} s_asset_end;

static void asset_end_record(uint8_t xfer, const char *reason, const char *id, uint32_t bytes)
{
    s_asset_end.xfer = xfer;
    s_asset_end.reason = reason;
    s_asset_end.bytes = bytes;
    snprintf(s_asset_end.id, sizeof s_asset_end.id, "%s", id ? id : "");
}

static struct {
    uint8_t xfer;
    const char *failure;
} s_photo_end;

static td_photo_pace_t s_pace;

/* Any bulk open, v1 or v2, ends the replay of the last Asset or picture end. */
static void bulk_opened(void)
{
    s_asset_end.xfer = 0;
    s_photo_end.xfer = 0;
}

/* Frees the buffer and forgets the transfer, and leaves a running update alone. */
static void drop_xfer(void)
{
    if (s.kind == KIND_ASSET_PUT) {
        td_asset_upload_abort();
    }
    heap_caps_free(s.buf);
    s.buf = NULL;
    s.fill = 0;
    s.kind = KIND_NONE;
}

/* A transfer that is replaced or cut. An update that was still receiving returns to IDLE. */
static void close_xfer(void)
{
    if (s.kind == KIND_UPDATE) {
        td_update_cancel();
    }
    drop_xfer();
}

static bool open_xfer(kind_t kind, uint32_t bytes)
{
    close_xfer();
    bulk_opened();
    s.buf = heap_caps_malloc(bytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (s.buf == NULL) {
        s.buf = heap_caps_malloc(bytes, MALLOC_CAP_8BIT);
    }
    if (s.buf == NULL) {
        return false;
    }
    s.kind = kind;
    s.last_id = s.last_id % 255 + 1;
    s.last_ms = (uint32_t)(esp_timer_get_time() / 1000);
    td_link_transfer_claim();
    return true;
}

/* An end request for a transfer that another carrier connection opened finds no transfer. */
static bool other_origin(void)
{
    return s.kind != KIND_NONE && !td_link_transfer_ours();
}

static bool put_sink(void *ctx, const uint8_t *data, size_t len)
{
    (void)ctx;
    memcpy(s.buf + s.fill, data, len);
    s.fill += len;
    return true;
}

static bool update_sink(void *ctx, const uint8_t *data, size_t len)
{
    (void)ctx;
    return td_update_write(data, len);
}

static bool u32_arg(const cJSON *request, const char *name, uint32_t *out)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(request, name);
    if (!cJSON_IsNumber(item) || item->valuedouble < 0 || item->valuedouble > 4294967295.0
        || item->valuedouble != floor(item->valuedouble)) {
        return false;
    }
    *out = (uint32_t)item->valuedouble;
    return true;
}

static const char *orientation_arg(const cJSON *request)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(request, "orientation");
    if (cJSON_IsString(item)) {
        if (strcmp(item->valuestring, "landscape") == 0) {
            return "landscape";
        }
        if (strcmp(item->valuestring, "portrait") == 0) {
            return "portrait";
        }
    }
    return NULL;
}

static bool icon_id_valid(const char *id)
{
    if (strlen(id) != ICON_ID_LEN) {
        return false;
    }
    for (size_t i = 0; i < ICON_ID_LEN; i++) {
        if (!((id[i] >= '0' && id[i] <= '9') || (id[i] >= 'a' && id[i] <= 'f'))) {
            return false;
        }
    }
    return true;
}

static const char *icon_id_arg(const cJSON *request)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(request, "id");
    return cJSON_IsString(item) && icon_id_valid(item->valuestring) ? item->valuestring : NULL;
}

void td_link_layout_fields(cJSON *reply)
{
    ui_layout_info_t info;
    ui_layout_info(&info);
    cJSON_AddStringToObject(reply, "source", info.from_storage ? "storage" : "builtin");
    cJSON_AddStringToObject(reply, "name", info.name);
    cJSON_AddStringToObject(reply, "orientation", info.orientation);
    cJSON_AddNumberToObject(reply, "pages", info.pages);
    cJSON_AddNumberToObject(reply, "tiles", info.tiles);
    cJSON_AddNumberToObject(reply, "icons_loaded", info.icons_loaded);
    cJSON_AddNumberToObject(reply, "icons_missing", info.icons_missing);
    cJSON_AddNumberToObject(reply, "bytes", info.bytes);
    cJSON_AddNumberToObject(reply, "load_ms", info.load_ms);
    cJSON_AddStringToObject(reply, "error", info.error);
}

void td_link_layout_put(const cJSON *request, cJSON *reply)
{
    const char *orientation = orientation_arg(request);
    uint32_t bytes;
    uint32_t crc32;
    if (orientation == NULL) {
        td_link_fail(reply, "orientation must be landscape or portrait");
    } else if (!u32_arg(request, "bytes", &bytes) || !u32_arg(request, "crc32", &crc32)) {
        td_link_fail(reply, "layout.put needs bytes and crc32");
    } else if (bytes == 0 || bytes > TD_LAYOUT_MAX_BYTES) {
        td_link_fail(reply, "layout must be 1 to 32768 bytes");
    } else if (!td_store_mounted()) {
        td_link_fail(reply, "store not mounted");
    } else if (!open_xfer(KIND_LAYOUT_PUT, bytes)) {
        td_link_fail(reply, "no memory");
    } else {
        td_xfer_init(&s.x, s.last_id, false, bytes, crc32, put_sink, NULL);
        snprintf(s.orientation, sizeof s.orientation, "%s", orientation);
        td_layout_path(orientation, s.path, sizeof s.path);
        cJSON_AddNumberToObject(reply, "xfer", s.x.id);
    }
}

void td_link_icon_put(const cJSON *request, cJSON *reply)
{
    const char *id = icon_id_arg(request);
    uint32_t bytes;
    uint32_t crc32;
    if (id == NULL) {
        td_link_fail(reply, "id must be 16 lowercase hex digits");
    } else if (!u32_arg(request, "bytes", &bytes) || !u32_arg(request, "crc32", &crc32)) {
        td_link_fail(reply, "icon.put needs bytes and crc32");
    } else if (bytes != ICON_FILE_BYTES) {
        td_link_fail(reply, "an icon is 12300 bytes");
    } else if (!td_store_mounted()) {
        td_link_fail(reply, "store not mounted");
    } else if (!open_xfer(KIND_ICON_PUT, bytes)) {
        td_link_fail(reply, "no memory");
    } else {
        td_xfer_init(&s.x, s.last_id, false, bytes, crc32, put_sink, NULL);
        snprintf(s.icon_id, sizeof s.icon_id, "%s", id);
        snprintf(s.path, sizeof s.path, ICON_DIR "/%s.bin", id);
        cJSON_AddNumberToObject(reply, "xfer", s.x.id);
    }
}

void td_link_layout_get(const cJSON *request, cJSON *reply)
{
    const char *orientation = orientation_arg(request);
    if (orientation == NULL) {
        td_link_fail(reply, "orientation must be landscape or portrait");
        return;
    }
    char path[TD_STORE_PATH_MAX];
    td_layout_path(orientation, path, sizeof path);
    td_store_stat_t stat;
    char message[64];
    if (td_store_stat(path, &stat) != ESP_OK || stat.is_dir) {
        snprintf(message, sizeof message, "no stored layout for %s", orientation);
        td_link_fail(reply, message);
        return;
    }
    if (stat.size == 0 || stat.size > TD_LAYOUT_MAX_BYTES) {
        td_link_fail(reply, "the stored layout is not a valid size");
        return;
    }
    if (!open_xfer(KIND_GET, stat.size)) {
        td_link_fail(reply, "no memory");
        return;
    }
    size_t len = 0;
    if (td_store_read(path, s.buf, stat.size, &len) != ESP_OK || len != stat.size) {
        close_xfer();
        td_link_fail(reply, "read failed");
        return;
    }
    const uint32_t crc32 = td_xfer_crc32(s.buf, len);
    td_xfer_init(&s.x, s.last_id, true, (uint32_t)len, crc32, NULL, NULL);
    cJSON_AddNumberToObject(reply, "xfer", s.x.id);
    cJSON_AddNumberToObject(reply, "bytes", (double)len);
    cJSON_AddNumberToObject(reply, "crc32", (double)crc32);
}

static bool le16_is(const uint8_t *p, unsigned value)
{
    return (unsigned)(p[0] | (p[1] << 8)) == value;
}

static void finish_put(cJSON *reply)
{
    char message[sizeof(td_layout_error_t) + 4];
    if (s.x.received != s.x.bytes) {
        snprintf(message, sizeof message, "received %u of %u bytes", (unsigned)s.x.received, (unsigned)s.x.bytes);
        td_link_fail(reply, message);
        return;
    }
    if (!td_xfer_complete(&s.x)) {
        td_link_fail(reply, "crc32 mismatch");
        return;
    }
    if (s.kind == KIND_ICON_PUT) {
        const uint8_t *h = s.buf;
        if (memcmp(h, "TDI1", 4) != 0 || !le16_is(h + 4, 64) || !le16_is(h + 6, 64) || !le16_is(h + 8, 1)
            || !le16_is(h + 10, 0)) {
            td_link_fail(reply, "bad icon header");
            return;
        }
        if (td_store_write_atomic(s.path, s.buf, s.fill) != ESP_OK) {
            td_link_fail(reply, "write failed");
            return;
        }
        cJSON_AddStringToObject(reply, "id", s.icon_id);
        return;
    }
    /* A candidate layout is validated before it replaces anything, so a bad file never costs the good one. */
    td_layout_t *parsed = NULL;
    td_layout_error_t err;
    if (!td_layout_parse((const char *)s.buf, s.fill, &parsed, &err)) {
        snprintf(message, sizeof message, "%s: %s", err.key, err.message);
        if (strcmp(err.key, "layout.live_image_limit") == 0) {
            if (td_link_v2_selected()) td_link_v2_fail(reply, "size", message);
            else td_link_fail(reply, "schema 2 requires v2 negotiation");
        }
        else td_link_fail(reply, message);
        return;
    }
    if (parsed->schema == 2 && !td_link_v2_selected()) {
        td_layout_free(parsed);
        td_link_fail(reply, "schema 2 requires v2 negotiation");
        return;
    }
    td_store_lock();
    for (uint8_t p = 0; p < parsed->page_count; p++) {
        unsigned animated_tiles = 0;
        uint32_t animated_pixels = 0;
        for (uint8_t t = 0; t < parsed->pages[p].tile_count; t++) {
            const char *icon = parsed->pages[p].tiles[t].icon;
            td_asset_format_t format = TD_ASSET_FMT_NONE;
            const bool gif = parsed->pages[p].tiles[t].render == UI_RENDER_GIF;
#if !CONFIG_TD_GIF_CAPS
            if (gif) {
                td_store_unlock();
                td_layout_free(parsed);
                td_link_fail(reply, "GIF Tiles require animation capabilities");
                return;
            }
#endif
            if (icon && strncmp(icon, "sha256:", 7) == 0 &&
                (!td_asset_present(icon + 7, NULL, &format) ||
                 format != (gif ? TD_ASSET_FMT_GIF : TD_ASSET_FMT_TDI1))) {
                td_store_unlock();
                td_layout_free(parsed);
                td_link_fail(reply, "Layout references an absent or incompatible Asset");
                return;
            }
            if (gif) {
                uint16_t width = 0, height = 0;
                if (!icon || !td_asset_gif_canvas(icon + 7, &width, &height) ||
                    !ui_gif_budget_add(&animated_tiles, &animated_pixels, width, height,
                                       TD_GIF_PAGE_TILES, TD_GIF_PAGE_PIXELS)) {
                    td_store_unlock();
                    td_layout_free(parsed);
                    td_link_fail(reply, "This Page has too many animations. Remove an animated Tile.");
                    cJSON_AddStringToObject(reply, "reason", "size");
                    return;
                }
            }
        }
    }
    td_layout_free(parsed);
    const esp_err_t written = td_store_write_atomic(s.path, s.buf, s.fill);
    td_store_unlock();
    if (written != ESP_OK) {
        td_link_fail(reply, "write failed");
        return;
    }
    const bool shown = strcmp(s.orientation, ui_layout_slot()) == 0;
    close_xfer();
    ui_leave_recovery("layout.put");
    if (shown && ui_ready() && !ui_layout_reload(false, &err)) {
        snprintf(message, sizeof message, "%s: %s", err.key, err.message);
        td_link_fail(reply, message);
        return;
    }
    td_link_layout_fields(reply);
}

void td_link_xfer_end(const cJSON *request, cJSON *reply)
{
    uint32_t id;
    if (!u32_arg(request, "xfer", &id)) {
        td_link_fail(reply, "xfer.end needs xfer");
    } else if (other_origin()) {
        td_link_fail(reply, "no transfer with that xfer");
        return;
    } else if (s.kind == KIND_ASSET_PUT || s.kind == KIND_PHOTO_PUT) {
        /* A v1 end never finishes or cuts a v2 Asset or picture put. */
        const char *v2_end =
            s.kind == KIND_ASSET_PUT ? "end an Asset with v2.asset.end" : "end a picture with v2.photo.end";
        td_link_fail(reply, id == s.x.id ? v2_end : "no transfer with that xfer");
        return;
    } else if (s.kind == KIND_NONE || id != s.x.id) {
        td_link_fail(reply, "no transfer with that xfer");
    } else if (s.kind == KIND_UPDATE) {
        td_link_fail(reply, "end a firmware update with update.end");
        return;
    } else if (s.kind != KIND_GET) {
        finish_put(reply);
    }
    close_xfer();
}

static void fail_update(cJSON *reply, td_update_reason_t reason)
{
    td_link_fail(reply, td_update_reason_text(reason));
    cJSON_AddStringToObject(reply, "reason", td_update_reason_name(reason));
}

void td_link_update_begin(const cJSON *request, cJSON *reply)
{
    uint32_t bytes;
    const cJSON *sha256 = cJSON_GetObjectItemCaseSensitive(request, "sha256");
    if (!u32_arg(request, "bytes", &bytes) || !cJSON_IsString(sha256)) {
        td_link_fail(reply, "update.begin needs bytes and sha256");
        cJSON_AddStringToObject(reply, "reason", "args");
        return;
    }
    /* The update decides first, because it refuses a second begin while one is receiving and that must not cancel it. */
    const char *slot = NULL;
    const td_update_reason_t reason = td_update_begin(bytes, sha256->valuestring, &slot);
    if (reason != TD_UPDATE_OK) {
        fail_update(reply, reason);
        return;
    }
    drop_xfer();
    bulk_opened();
    s.kind = KIND_UPDATE;
    s.last_id = s.last_id % 255 + 1;
    s.last_ms = (uint32_t)(esp_timer_get_time() / 1000);
    td_link_transfer_claim();
    td_xfer_init(&s.x, s.last_id, false, bytes, 0, update_sink, NULL);
    cJSON_AddNumberToObject(reply, "xfer", s.x.id);
    cJSON_AddStringToObject(reply, "slot", slot);
}

void td_link_update_end(const cJSON *request, cJSON *reply)
{
    uint32_t id;
    if (!u32_arg(request, "xfer", &id)) {
        td_link_fail(reply, "update.end needs xfer");
        cJSON_AddStringToObject(reply, "reason", "args");
        return;
    }
    if (s.kind != KIND_UPDATE || id != s.x.id || other_origin()) {
        td_link_fail(reply, "no update with that xfer");
        cJSON_AddStringToObject(reply, "reason", "state");
        return;
    }
    const char *slot = NULL;
    const td_update_reason_t reason = td_update_end(&slot);
    if (reason == TD_UPDATE_OK) {
        cJSON_AddStringToObject(reply, "slot", slot);
    } else {
        fail_update(reply, reason);
    }
    drop_xfer();
}

static void send_chunk(uint16_t idx)
{
    static uint8_t payload[TD_FRAME_PAYLOAD_MAX];
    const size_t start = (size_t)idx * TD_FRAME_CHUNK_DATA_MAX;
    size_t len = s.x.bytes - start;
    if (len > TD_FRAME_CHUNK_DATA_MAX) {
        len = TD_FRAME_CHUNK_DATA_MAX;
    }
    payload[0] = s.x.id;
    payload[1] = idx & 0xff;
    payload[2] = idx >> 8;
    memcpy(payload + TD_FRAME_CHUNK_HEADER, s.buf + start, len);
    s.tx_seq = s.tx_seq % 255 + 1;
    td_link_send(TD_FRAME_CHUNK, s.tx_seq, 0, payload, TD_FRAME_CHUNK_HEADER + len);
}

static bool photo_cancelled(void)
{
    uint32_t entry = 0;
    if (s.kind != KIND_PHOTO_PUT || (power_idle_face(&entry) && entry == s.photo_entry)) {
        return false;
    }
    s_photo_end.xfer = s.x.id;
    s_photo_end.failure = "idle";
    drop_xfer();
    return true;
}

void td_link_files_chunk(uint8_t seq, const uint8_t *payload, size_t len)
{
    if (len < TD_FRAME_CHUNK_HEADER
        || (s.kind != KIND_LAYOUT_PUT && s.kind != KIND_ICON_PUT && s.kind != KIND_UPDATE && s.kind != KIND_ASSET_PUT
            && s.kind != KIND_PHOTO_PUT)
        || payload[0] != s.x.id) {
        td_link_count_drop();
        return;
    }
    if (photo_cancelled()) {
        return;
    }
    const uint16_t idx = (uint16_t)(payload[1] | (payload[2] << 8));
    /* A duplicate, a gap and a chunk past the window all get the same answer: the next index the deck wants. */
    td_xfer_receive(&s.x, idx, payload + TD_FRAME_CHUNK_HEADER, len - TD_FRAME_CHUNK_HEADER);
    s.last_ms = (uint32_t)(esp_timer_get_time() / 1000);
    if (s.kind == KIND_ASSET_PUT && s.asset_failure != NULL) {
        /* A failed sink ends the transfer and stops the acks. v2.asset.end reports why. */
        asset_end_record(s.x.id, s.asset_failure, NULL, 0);
        drop_xfer();
        return;
    }
    const uint8_t ack[3] = { s.x.id, s.x.next & 0xff, s.x.next >> 8 };
    td_link_send(TD_FRAME_ACK, seq, TD_FRAME_RESPONSE, ack, sizeof ack);
}

void td_link_files_ack(const uint8_t *payload, size_t len)
{
    if (len != 3 || s.kind != KIND_GET || payload[0] != s.x.id) {
        td_link_count_drop();
        return;
    }
    td_xfer_ack(&s.x, (uint16_t)(payload[1] | (payload[2] << 8)));
    s.last_ms = (uint32_t)(esp_timer_get_time() / 1000);
}

void td_link_files_service(uint32_t now_ms)
{
    if (s.kind == KIND_NONE || photo_cancelled()) {
        return;
    }
    if (!td_link_transfer_live()) {
        /* The carrier connection that opened it ended: a network session closed. */
        close_xfer();
        return;
    }
    if (s.kind == KIND_GET) {
        int idx;
        for (int guard = 0; guard < 2 * TD_XFER_WINDOW && (idx = td_xfer_due(&s.x, now_ms)) >= 0; guard++) {
            send_chunk((uint16_t)idx);
        }
        if (s.x.aborted || now_ms - s.last_ms > GET_IDLE_MS) {
            close_xfer();
        }
    } else if (now_ms - s.last_ms > TD_XFER_IDLE_MS) {
        close_xfer();
    }
}

typedef struct {
    uint32_t skip;
    uint32_t seen;
    uint32_t count;
    bool more;
    cJSON *ids;
} icon_list_t;

static bool icon_name_valid(const char *name)
{
    char id[ICON_ID_LEN + 1];
    if (strlen(name) != ICON_ID_LEN + 4 || strcmp(name + ICON_ID_LEN, ".bin") != 0) {
        return false;
    }
    memcpy(id, name, ICON_ID_LEN);
    id[ICON_ID_LEN] = '\0';
    return icon_id_valid(id);
}

static bool icon_list_cb(void *ctx, const char *name, uint32_t size, bool is_dir)
{
    icon_list_t *list = ctx;
    (void)size;
    if (is_dir || !icon_name_valid(name) || list->seen++ < list->skip) {
        return true;
    }
    if (list->count == ICON_PAGE) {
        list->more = true;
        return false;
    }
    char id[ICON_ID_LEN + 1];
    memcpy(id, name, ICON_ID_LEN);
    id[ICON_ID_LEN] = '\0';
    cJSON_AddItemToArray(list->ids, cJSON_CreateString(id));
    list->count++;
    return true;
}

void td_link_icon_list(const cJSON *request, cJSON *reply)
{
    const cJSON *cursor = cJSON_GetObjectItemCaseSensitive(request, "cursor");
    icon_list_t list = { .ids = cJSON_CreateArray() };
    if (list.ids == NULL) {
        td_link_fail(reply, "no memory");
        return;
    }
    if (cJSON_IsNumber(cursor) && cursor->valuedouble >= 0 && cursor->valuedouble < 1e9) {
        list.skip = (uint32_t)cursor->valuedouble;
    }
    td_store_list(ICON_DIR, icon_list_cb, &list);
    cJSON_AddItemToObject(reply, "ids", list.ids);
    if (list.more) {
        cJSON_AddNumberToObject(reply, "next", list.skip + list.count);
    } else {
        cJSON_AddNullToObject(reply, "next");
    }
}

void td_link_icon_rm(const cJSON *request, cJSON *reply)
{
    const char *id = icon_id_arg(request);
    if (id == NULL) {
        td_link_fail(reply, "id must be 16 lowercase hex digits");
        return;
    }
    char path[TD_STORE_PATH_MAX];
    snprintf(path, sizeof path, ICON_DIR "/%s.bin", id);
    esp_err_t err = td_store_remove(path);
    if (err == ESP_ERR_NOT_FOUND) {
        td_link_fail(reply, "no such icon");
    } else if (err != ESP_OK) {
        td_link_fail(reply, "remove failed");
    }
}

static void settings_fields(cJSON *reply, const td_settings_t *settings)
{
    cJSON_AddNumberToObject(reply, "idle_timeout_s", settings->idle_timeout_s);
    cJSON_AddStringToObject(reply, "orientation", td_settings_orientation_name(settings->orientation));
}

void td_link_settings_get(const cJSON *request, cJSON *reply)
{
    (void)request;
    td_settings_t settings;
    td_store_settings_load(&settings);
    settings_fields(reply, &settings);
}

void td_link_settings_put(const cJSON *request, cJSON *reply)
{
    td_settings_t before;
    td_store_settings_load(&before);
    td_settings_t next = before;
    const cJSON *idle = cJSON_GetObjectItemCaseSensitive(request, "idle_timeout_s");
    const cJSON *orientation = cJSON_GetObjectItemCaseSensitive(request, "orientation");
    if (idle != NULL) {
        uint32_t seconds;
        if (!u32_arg(request, "idle_timeout_s", &seconds) || seconds < TD_SETTINGS_IDLE_MIN_S
            || seconds > TD_SETTINGS_IDLE_MAX_S) {
            td_link_fail(reply, "idle_timeout_s must be a whole number from 1 to 86400");
            return;
        }
        next.idle_timeout_s = seconds;
    }
    if (orientation != NULL
        && (!cJSON_IsString(orientation) || !td_settings_orientation_parse(orientation->valuestring, &next.orientation))) {
        td_link_fail(reply, "orientation must be landscape or portrait");
        return;
    }
    if (td_store_settings_save(&next) != ESP_OK) {
        td_link_fail(reply, "settings not saved");
        return;
    }
    ui_leave_recovery("settings.put");
    power_set_idle_timeout_s(next.idle_timeout_s, esp_timer_get_time());
    settings_fields(reply, &next);
    /* Against the orientation on screen, which recovery mode runs at the default and not at the stored one. */
    cJSON_AddBoolToObject(reply, "reboot_required",
                          strcmp(td_settings_orientation_name(next.orientation), ui_layout_slot()) != 0);
}

static bool count_cb(void *ctx, const char *name, uint32_t size, bool is_dir)
{
    (void)name;
    (void)size;
    if (!is_dir) {
        ++*(uint32_t *)ctx;
    }
    return true;
}

void td_link_store_stat(const cJSON *request, cJSON *reply)
{
    (void)request;
    td_store_info_t info;
    if (td_store_info(&info) != ESP_OK) {
        td_link_fail(reply, "store not mounted");
        return;
    }
    uint32_t layouts = 0;
    uint32_t icons = 0;
    td_store_list(LAYOUT_DIR, count_cb, &layouts);
    td_store_list(ICON_DIR, count_cb, &icons);
    cJSON_AddNumberToObject(reply, "total_bytes", info.total_bytes);
    cJSON_AddNumberToObject(reply, "used_bytes", info.used_bytes);
    cJSON_AddNumberToObject(reply, "layouts", layouts);
    cJSON_AddNumberToObject(reply, "icons", icons);
}

static bool asset_sink(void *ctx, const uint8_t *data, size_t len)
{
    (void)ctx;
    const td_asset_result_t result = td_asset_upload_write(data, len);
    if (result != TD_ASSET_OK) {
        s.asset_failure = td_asset_result_reason(result);
        return false;
    }
    return true;
}

void td_link_files_select(void)
{
    if (s.kind == KIND_ASSET_PUT || s.kind == KIND_PHOTO_PUT) {
        drop_xfer();
    }
}

static const char *asset_id_field(const cJSON *request)
{
    const char *id = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "id"));
    return td_asset_id_valid(id) ? id : NULL;
}

void td_link_asset_stat(const cJSON *request, cJSON *reply)
{
    td_asset_space_t space;
    if (!td_link_v2_fields(request, NULL, 0)) {
        td_link_v2_fail(reply, "args", "Invalid request fields");
    } else if (td_asset_space(&space) != TD_ASSET_OK) {
        td_link_v2_fail(reply, "storage", "Storage unavailable");
    } else {
        cJSON_AddNumberToObject(reply, "total_bytes", space.total_bytes);
        cJSON_AddNumberToObject(reply, "used_bytes", space.used_bytes);
        cJSON_AddNumberToObject(reply, "reserved_bytes", space.reserved_bytes);
        cJSON_AddNumberToObject(reply, "free_bytes", space.free_bytes);
    }
}

void td_link_asset_list(const cJSON *request, cJSON *reply)
{
    static const char *const keys[] = { "after" };
    const cJSON *after = cJSON_GetObjectItemCaseSensitive(request, "after");
    if (!td_link_v2_fields(request, keys, 1) || after == NULL
        || !(cJSON_IsNull(after) || td_asset_id_valid(cJSON_GetStringValue(after)))) {
        td_link_v2_fail(reply, "args", "Invalid request fields");
        return;
    }
    td_asset_item_t items[TD_ASSET_PAGE];
    size_t count = 0;
    bool more = false;
    if (td_asset_list(cJSON_GetStringValue(after), items, &count, &more) != TD_ASSET_OK) {
        td_link_v2_fail(reply, "storage", "Storage unavailable");
        return;
    }
    cJSON *list = cJSON_AddArrayToObject(reply, "items");
    for (size_t i = 0; list != NULL && i < count; i++) {
        cJSON *item = cJSON_CreateObject();
        cJSON_AddStringToObject(item, "id", items[i].id);
        cJSON_AddNumberToObject(item, "bytes", items[i].bytes);
        cJSON_AddStringToObject(item, "format", td_asset_format_name(items[i].format));
        cJSON_AddItemToArray(list, item);
    }
    if (more && count > 0) {
        cJSON_AddStringToObject(reply, "next", items[count - 1].id);
    } else {
        cJSON_AddNullToObject(reply, "next");
    }
}

void td_link_asset_put(const cJSON *request, cJSON *reply)
{
    static const char *const keys[] = { "id", "bytes", "format" };
    const char *id = asset_id_field(request);
    const char *format_name = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "format"));
    uint32_t bytes;
    if (!td_link_v2_fields(request, keys, 3) || id == NULL || format_name == NULL || !u32_arg(request, "bytes", &bytes)
        || bytes == 0) {
        td_link_v2_fail(reply, "args", "Invalid request fields");
        return;
    }
    const td_asset_format_t format = td_asset_format_parse(format_name);
    if (format == TD_ASSET_FMT_NONE) {
        td_link_v2_fail(reply, "format", "Unsupported Asset format");
        return;
    }
#if !CONFIG_TD_GIF_CAPS
    if (format == TD_ASSET_FMT_GIF) {
        td_link_v2_fail(reply, "format", "Unsupported Asset format");
        return;
    }
#endif
    if (bytes > (format == TD_ASSET_FMT_GIF ? TD_GIF_MAX_BYTES : format == TD_ASSET_FMT_JPEG ? TD_ASSET_JPEG_MAX_BYTES : TD_ASSET_MAX_BYTES)) {
        td_link_v2_fail(reply, "size", "Asset too large");
        return;
    }
    if (!td_store_mounted()) {
        td_link_v2_fail(reply, "storage", "Storage unavailable");
        return;
    }
    uint32_t stored_bytes;
    td_asset_format_t stored_format;
    if (td_asset_present(id, &stored_bytes, &stored_format)) {
        if (stored_bytes != bytes || stored_format != format) {
            td_link_v2_fail(reply, "format", "Asset differs from the stored one");
            return;
        }
        cJSON_AddBoolToObject(reply, "present", true);
        cJSON_AddNullToObject(reply, "xfer");
        return;
    }
    if (s.kind != KIND_NONE) {
        td_link_v2_fail(reply, "busy", "Another transfer is open");
        return;
    }
    const td_asset_result_t result = td_asset_upload_begin(id, bytes, format);
    if (result != TD_ASSET_OK) {
        td_link_v2_fail(reply, td_asset_result_reason(result),
                        result == TD_ASSET_FULL ? "Not enough free space" : "Storage unavailable");
        return;
    }
    bulk_opened();
    s.kind = KIND_ASSET_PUT;
    s.asset_failure = NULL;
    snprintf(s.asset_id, sizeof s.asset_id, "%s", id);
    s.last_id = s.last_id % 255 + 1;
    s.last_ms = (uint32_t)(esp_timer_get_time() / 1000);
    td_link_transfer_claim();
    td_xfer_init(&s.x, s.last_id, false, bytes, 0, asset_sink, NULL);
    cJSON_AddBoolToObject(reply, "present", false);
    cJSON_AddNumberToObject(reply, "xfer", s.x.id);
}

static void asset_end_reply(cJSON *reply)
{
    if (s_asset_end.reason != NULL) {
        td_link_v2_fail(reply, s_asset_end.reason, "Asset not stored");
        return;
    }
    cJSON_AddStringToObject(reply, "id", s_asset_end.id);
    cJSON_AddNumberToObject(reply, "bytes", s_asset_end.bytes);
}

void td_link_asset_end(const cJSON *request, cJSON *reply)
{
    static const char *const keys[] = { "xfer" };
    uint32_t id;
    if (!td_link_v2_fields(request, keys, 1) || !u32_arg(request, "xfer", &id) || id == 0 || id > 255) {
        td_link_v2_fail(reply, "args", "Invalid request fields");
        return;
    }
    if (s.kind == KIND_ASSET_PUT && s.x.id == id && !other_origin()) {
        const td_asset_result_t result = td_asset_upload_finish();
        if (result == TD_ASSET_INCOMPLETE) {
            td_link_v2_fail(reply, "incomplete", "Bytes are still missing");
            return;
        }
        asset_end_record(s.x.id, result == TD_ASSET_OK ? NULL : td_asset_result_reason(result), s.asset_id, s.x.bytes);
        drop_xfer();
        asset_end_reply(reply);
    } else if (s_asset_end.xfer != 0 && s_asset_end.xfer == id) {
        asset_end_reply(reply);
    } else {
        td_link_v2_fail(reply, "state", "No Asset transfer with that xfer");
    }
}

static const char *asset_in_use(const char *id)
{
    static const char *const orientations[] = { "landscape", "portrait" };
    char needle[sizeof "sha256:" + TD_ASSET_ID_LEN];
    snprintf(needle, sizeof needle, "sha256:%s", id);
    char *buf = NULL;
    const char *reason = NULL;
    for (size_t i = 0; i < 2 && reason == NULL; i++) {
        char path[TD_STORE_PATH_MAX];
        td_layout_path(orientations[i], path, sizeof path);
        td_store_stat_t stat;
        if (td_store_stat(path, &stat) == ESP_ERR_NOT_FOUND) {
            continue;
        }
        if (buf == NULL) {
            buf = heap_caps_malloc(TD_LAYOUT_MAX_BYTES + 1, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
        }
        size_t len = 0;
        if (buf == NULL) {
            reason = "memory";
        } else if (td_store_read(path, buf, TD_LAYOUT_MAX_BYTES, &len) != ESP_OK) {
            reason = "storage";
        } else {
            buf[len] = '\0';
            td_layout_t *layout = NULL;
            if (!td_layout_parse(buf, len, &layout, NULL)) {
                reason = "storage";
            } else {
                for (uint8_t p = 0; p < layout->page_count; p++) {
                    for (uint8_t t = 0; t < layout->pages[p].tile_count; t++) {
                        const char *asset = layout->pages[p].tiles[t].icon;
                        if (asset && strcmp(asset, needle) == 0) reason = "in-use";
                    }
                }
                td_layout_free(layout);
            }
        }
    }
    heap_caps_free(buf);
    return reason;
}

void td_link_asset_delete(const cJSON *request, cJSON *reply)
{
    static const char *const keys[] = { "id" };
    const char *id = asset_id_field(request);
    if (!td_link_v2_fields(request, keys, 1) || id == NULL) {
        td_link_v2_fail(reply, "args", "Invalid request fields");
        return;
    }
    const char *upload = td_asset_upload_id();
    if (upload != NULL && strcmp(upload, id) == 0) {
        td_link_v2_fail(reply, "busy", "The Asset is being uploaded");
        return;
    }
    if (!td_store_mounted()) {
        td_link_v2_fail(reply, "storage", "Storage unavailable");
        return;
    }
    bool deleted = false;
    td_store_lock();
    const char *reason = asset_in_use(id);
    if (reason == NULL) {
        td_asset_result_t result = td_asset_delete(id, &deleted);
        if (result != TD_ASSET_OK) reason = td_asset_result_reason(result);
    }
    td_store_unlock();
    if (reason != NULL) {
        td_link_v2_fail(reply, reason,
                        strcmp(reason, "in-use") == 0 ? "A stored Layout uses the Asset" : "Asset not deleted");
        return;
    }
    cJSON_AddBoolToObject(reply, "deleted", deleted);
}

void td_link_photo_put(const cJSON *request, cJSON *reply)
{
    static const char *const keys[] = { "bytes", "sha256" };
    const char *sha256 = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "sha256"));
    uint8_t digest[TD_SHA256_BYTES];
    uint32_t bytes;
    if (!td_link_v2_fields(request, keys, 2) || !u32_arg(request, "bytes", &bytes) || sha256 == NULL
        || !td_sha256_from_hex(sha256, digest)) {
        td_link_v2_fail(reply, "args", "Invalid request fields");
        return;
    }
    uint32_t entry = 0;
    const bool idle = power_idle_face(&entry);
    uint32_t retry_ms = 0;
    switch (td_photo_admit(&s_pace, bytes, TD_PHOTO_BYTES_MAX, idle, entry, td_link_idle_interval_s(),
                           esp_timer_get_time(), &retry_ms)) {
    case TD_PHOTO_SIZE:
        td_link_v2_fail(reply, "size", "A picture is 1 to 131072 bytes");
        return;
    case TD_PHOTO_IDLE:
        td_link_v2_fail(reply, "idle", "The Device is awake or the photo frame is off");
        return;
    case TD_PHOTO_RATE:
        td_link_v2_fail(reply, "rate", "The next picture is not due yet");
        cJSON_AddNumberToObject(reply, "retry_ms", retry_ms);
        return;
    case TD_PHOTO_OK:
        break;
    }
    if (s.kind != KIND_NONE) {
        td_link_v2_fail(reply, "busy", "Another transfer is open");
        return;
    }
    if (!open_xfer(KIND_PHOTO_PUT, bytes)) {
        td_link_v2_fail(reply, "memory", "No memory for the picture");
        return;
    }
    td_xfer_init(&s.x, s.last_id, false, bytes, 0, put_sink, NULL);
    memcpy(s.photo_sha256, digest, sizeof digest);
    s.photo_entry = entry;
    cJSON_AddNumberToObject(reply, "xfer", s.x.id);
}

static const char *photo_show_failure(void)
{
    uint32_t entry = 0;
    if (!power_idle_face(&entry) || entry != s.photo_entry) {
        return "idle";
    }
    td_sha256_t sha;
    uint8_t digest[TD_SHA256_BYTES];
    td_sha256_init(&sha);
    td_sha256_update(&sha, s.buf, s.fill);
    td_sha256_final(&sha, digest);
    if (memcmp(digest, s.photo_sha256, sizeof digest) != 0) {
        return "hash";
    }
    switch (ui_idle_picture(s.buf, s.fill)) {
    case UI_PICTURE_SHOWN:
        td_photo_shown(&s_pace, entry, esp_timer_get_time());
        return NULL;
    case UI_PICTURE_SIZE:
        return "size";
    case UI_PICTURE_IDLE:
        return "idle";
    case UI_PICTURE_MEMORY:
        return "memory";
    case UI_PICTURE_FORMAT:
    default:
        return "format";
    }
}

static void photo_end_reply(cJSON *reply)
{
    static const struct {
        const char *reason;
        const char *error;
    } errors[] = {
        { "idle", "The Device woke up" },
        { "hash", "SHA-256 differs" },
        { "size", "The picture is larger than the screen" },
        { "memory", "No memory for the picture" },
        { "format", "Not a baseline JPEG" },
    };
    if (s_photo_end.failure == NULL) {
        cJSON_AddBoolToObject(reply, "shown", true);
        return;
    }
    const char *error = "Picture not shown";
    for (size_t i = 0; i < sizeof errors / sizeof errors[0]; i++) {
        if (strcmp(errors[i].reason, s_photo_end.failure) == 0) {
            error = errors[i].error;
        }
    }
    td_link_v2_fail(reply, s_photo_end.failure, error);
}

void td_link_photo_end(const cJSON *request, cJSON *reply)
{
    static const char *const keys[] = { "xfer" };
    uint32_t id;
    if (!td_link_v2_fields(request, keys, 1) || !u32_arg(request, "xfer", &id) || id == 0 || id > 255) {
        td_link_v2_fail(reply, "args", "Invalid request fields");
        return;
    }
    if (s.kind == KIND_PHOTO_PUT && s.x.id == id && !other_origin() && !photo_cancelled()) {
        if (s.x.received != s.x.bytes) {
            td_link_v2_fail(reply, "incomplete", "Bytes are still missing");
            return;
        }
        s_photo_end.xfer = s.x.id;
        s_photo_end.failure = photo_show_failure();
        drop_xfer();
        photo_end_reply(reply);
    } else if (s_photo_end.xfer != 0 && s_photo_end.xfer == id) {
        photo_end_reply(reply);
    } else {
        td_link_v2_fail(reply, "state", "No picture transfer with that xfer");
    }
}
