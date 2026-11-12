#include "td_layout.h"
#include "td_asset.h"
#include "ui_gif_player.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

#define CONFIG_TD_GIF_CAPS 1
#define ESP_OK 0
typedef int esp_err_t;
typedef struct { bool failed; const char *reason; } cJSON;
enum { TD_TRANSFER_ICON_PUT = 1 };
typedef struct {
    struct { unsigned received, bytes; } x;
    unsigned kind, fill;
    uint8_t *buf;
} td_transfer_t;
static td_transfer_t s;
static struct { const char *path, *orientation, *icon_id; } s_req;
static ui_tile_t tiles[5];
static ui_page_t pages[2];
static td_layout_t layout;
static unsigned committed;
static unsigned candidate;
static bool selected = true, parse_budget_error;
static void td_link_fail(cJSON *reply, const char *message) { (void)message; reply->failed = true; }
static bool td_xfer_complete(const void *x) { (void)x; return true; }
static bool le16_is(const uint8_t *p, unsigned value) { return (unsigned)(p[0] | p[1] << 8) == value; }
static esp_err_t td_store_write_atomic(const char *path, const uint8_t *data, unsigned size)
{
    (void)path; (void)data; (void)size; committed = candidate; return ESP_OK;
}
static void cJSON_AddStringToObject(cJSON *reply, const char *name, const char *value)
{
    if (!strcmp(name, "reason")) reply->reason = value;
}
static void td_link_v2_fail(cJSON *reply, const char *reason, const char *message)
{ td_link_fail(reply, message); reply->reason = reason; }
bool td_layout_parse(const char *data, size_t bytes, td_layout_t **out, td_layout_error_t *error)
{
    (void)data; (void)bytes;
    if (parse_budget_error) { memcpy(error->key, "layout.live_image_limit", sizeof "layout.live_image_limit"); return false; }
    *out = &layout; return true;
}
void td_layout_free(td_layout_t *p) { (void)p; }
static bool td_link_v2_selected(void) { return selected; }
static void td_store_lock(void) {}
static void td_store_unlock(void) {}
bool td_asset_present(const char *id, uint32_t *bytes, td_asset_format_t *format)
{
    (void)id; (void)bytes; *format = TD_ASSET_FMT_GIF; return true;
}
static uint16_t canvas_width = 128, canvas_height = 128;
bool td_asset_gif_canvas(const char *id, uint16_t *width, uint16_t *height)
{
    (void)id; *width = canvas_width; *height = canvas_height; return true;
}
static const char *ui_layout_slot(void) { return "portrait"; }
static void td_transfer_close(td_transfer_t *t) { (void)t; }
static void ui_leave_recovery(const char *why) { (void)why; }
static bool ui_ready(void) { return false; }
static bool ui_layout_reload(bool builtin, td_layout_error_t *err) { (void)builtin; (void)err; return true; }
static void td_link_layout_fields(cJSON *reply) { (void)reply; }
#include "finish_put.c"

int main(void)
{
    for (unsigned i = 0; i < 5; i++) tiles[i] = (ui_tile_t){.render = UI_RENDER_GIF, .icon = "sha256:reused"};
    pages[0] = (ui_page_t){.tiles = tiles, .tile_count = 4};
    pages[1] = (ui_page_t){.tiles = tiles, .tile_count = 4};
    layout = (td_layout_t){.schema = 2, .pages = pages, .page_count = 2};
    uint8_t data[12] = {0};
    s.buf = data; s.fill = sizeof data; s.x.received = s.x.bytes = sizeof data;
    s_req.orientation = "landscape";
    candidate = 1;
    cJSON reply = {0};
    finish_put(&s, &reply);
    assert(!reply.failed && committed == 1);
    candidate = 2;
    pages[1].tile_count = 5;
    reply = (cJSON){0};
    finish_put(&s, &reply);
    assert(reply.failed && reply.reason && !strcmp(reply.reason, "size") && committed == 1);
    pages[1].tile_count = 4;
    reply = (cJSON){0};
    finish_put(&s, &reply);
    assert(!reply.failed && committed == 2);
    candidate = 3;
    pages[0].tile_count = pages[1].tile_count = 3;
    canvas_width = canvas_height = 160;
    reply = (cJSON){0};
    finish_put(&s, &reply);
    assert(reply.failed && reply.reason && !strcmp(reply.reason, "size") && committed == 2);
    candidate = 4;
    pages[0].tile_count = pages[1].tile_count = 2;
    canvas_width = 256;
    canvas_height = 128;
    reply = (cJSON){0};
    finish_put(&s, &reply);
    assert(!reply.failed && committed == 4);
    puts("Layout commit counts reused Assets per Tile, resets each Page, refuses size by Tiles or pixels and retains stored Layout: OK");
    selected = false; parse_budget_error = true;
    reply = (cJSON){0};
    finish_put(&s, &reply);
    assert(reply.failed && !reply.reason && committed == 4);
    return 0;
}
