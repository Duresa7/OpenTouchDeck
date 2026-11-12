#include <assert.h>
#include <string.h>
#include "ui_pages.h"
#include "td_ble_hid.h"

static unsigned count, link_count;
static uint8_t bytes[4][8];
static size_t lengths[4];
static td_ble_input_t inputs[4];
static void report(void *ctx, const key_report_t *r)
{
    (void)ctx;
    assert(count < 4);
    lengths[count] = td_ble_encode(r, 1, bytes[count], &inputs[count]);
    count++;
}
static void bluetooth(void *ctx, const td_action_t *action)
{
    (void)ctx;
    ui_action_bluetooth(action, (key_sink_t){ .report = report });
}
static void link(void *ctx, uint8_t usage, uint8_t mods)
{
    (void)ctx; (void)usage; (void)mods;
    link_count++;
}
int main(void)
{
    ui_tile_t tile = { .action = { .type = TD_ACTION_KEY, .bluetooth = true, .key = { .usage = 6, .mods = 3 } } };
    const ui_action_ops_t ops = { .bluetooth = bluetooth, .key = link };
    assert(ui_tile_click(&tile, &ops, NULL));
    assert(count == 2 && link_count == 0);
    assert(lengths[0] == 8 && inputs[0] == TD_BLE_IN_KEYBOARD);
    assert(bytes[0][0] == 3 && bytes[0][2] == 6);
    assert(memcmp(bytes[1], (uint8_t[8]){0}, 8) == 0);
    tile.action = (td_action_t){ .type = TD_ACTION_CONSUMER, .bluetooth = true, .consumer = { .usage = 205 } };
    assert(ui_tile_click(&tile, &ops, NULL));
    assert(count == 4 && link_count == 0);
    assert(inputs[2] == TD_BLE_IN_CONSUMER && lengths[2] == 2);
    assert(bytes[2][0] == 205 && bytes[2][1] == 0);
    assert(bytes[3][0] == 0 && bytes[3][1] == 0);
    tile.action = (td_action_t){ .type = TD_ACTION_KEY, .key = { .usage = 58 } };
    assert(ui_tile_click(&tile, &ops, NULL));
    assert(count == 4 && link_count == 1);
    return 0;
}
