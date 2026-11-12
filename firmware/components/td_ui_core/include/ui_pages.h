#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "key_engine.h"
#include "key_sink.h"

#ifdef __cplusplus
extern "C" {
#endif

#define UI_GRID_MAX 8

typedef enum {
    TD_ACTION_NONE = 0,
    TD_ACTION_KEY,
    TD_ACTION_MOD,
    TD_ACTION_CONSUMER,
    TD_ACTION_PAGE,
    TD_ACTION_APP,
    TD_ACTION_WIDGET,
} td_action_type_t;

/* The app-side actions. The deck never runs them, it only reports the press. */
typedef enum {
    TD_APP_LAUNCH,
    TD_APP_OPEN,
    TD_APP_COMMAND,
} td_app_kind_t;

/*
 * What a tile does, from its layout. NONE is 0, so a tile with no action does nothing. A key carries the HID
 * modifier byte of its own `mods`, Ctrl 1, Shift 2, Alt 4, Win 8. A page action holds the index of its target page.
 * The widget strings point into the layout that owns the tile.
 */
typedef struct {
    td_action_type_t type;
    bool bluetooth; /* key and consumer only; false keeps the Companion link */
    union {
        struct {
            uint8_t usage;
            uint8_t mods;
        } key;
        struct {
            key_mod_t mod;
        } mod;
        struct {
            uint16_t usage;
        } consumer;
        struct {
            uint8_t index;
        } page;
        struct {
            td_app_kind_t kind;
        } app;
        struct {
            const char *source;
            const char *field;
            const char *account;
        } widget;
    };
} td_action_t;

/* The schema's name of the action type: "none", "key", "mod", "consumer", "page", "launch", "open", "command" or
 * "widget". */
const char *td_action_name(const td_action_t *action);

/* How a tile draws. A schema 1 tile, and a schema 2 tile without `render`, is static. */
typedef enum {
    UI_RENDER_STATIC = 0,
    UI_RENDER_GIF,
    UI_RENDER_GRAPH,
    UI_RENDER_BAR,
    UI_RENDER_NOW_PLAYING,
} ui_render_t;

/* The value type of a Live tile's stream, as the layout declares it and `v2.live.set` sends it. */
typedef enum {
    UI_STREAM_NUMBER = 0,
    UI_STREAM_SERIES,
    UI_STREAM_TEXT,
    UI_STREAM_IMAGE,
} ui_stream_type_t;

/* One stream of a Live tile: its id, unique on the tile, and its type. */
typedef struct {
    const char *id;
    ui_stream_type_t type;
} ui_stream_t;

/*
 * One tile on a page's grid. A tile covers row_span by col_span cells from its top-left cell. `id` is the stable name
 * that the console and the layouts use. It is unique across every page. `label` is what the tile shows. `icon` is
 * NULL, "lv:<name>" or "file:<16 hex digits>", as the layout wrote it.
 */
typedef struct {
    const char *id;
    const char *label;
    uint8_t row;
    uint8_t col;
    uint8_t row_span;
    uint8_t col_span;
    const char *icon;
    td_action_t action;
    ui_render_t render;
    bool input;
    uint8_t stream_count;
    const ui_stream_t *streams;
} ui_tile_t;

typedef enum {
    UI_PAGE_GRID = 0,
    UI_PAGE_DASHBOARD,
} ui_page_kind_t;

/* One swipeable page: a rows by cols grid and the tiles on it. `id` is unique across the pages. A dashboard page is a
 * 1 by 1 grid with no tiles, and `source` names the app source that fills it. */
typedef struct {
    const char *id;
    const char *title;
    uint8_t rows;
    uint8_t cols;
    uint8_t tile_count;
    const ui_tile_t *tiles;
    ui_page_kind_t kind;
    const char *source;
} ui_page_t;

typedef struct {
    int16_t x;
    int16_t y;
    int16_t w;
    int16_t h;
} ui_rect_t;

/*
 * True when the page is safe to draw. The grid is 1 to UI_GRID_MAX in each direction, the page's id and title
 * and every tile's id and label are non-empty, every tile has a span of at least 1 and stays inside the grid,
 * and no two tiles share a cell. It checks one page, so an id that two pages share is not caught here.
 */
bool ui_page_valid(const ui_page_t *page);

/*
 * Where a tile sits when the page's grid fills `area`, with `gap` pixels between neighbouring cells. Cell edges
 * come from integer division of the whole area, so every cell is within one pixel of the same size, the gaps are
 * exactly `gap`, and the last column and row end on the area's right and bottom edge. A spanning tile runs from
 * the first edge of its first cell to the last edge of its last cell, so it covers the gaps between its cells.
 * Returns an empty rect (w and h of 0) for a tile that does not fit a valid grid.
 */
ui_rect_t ui_tile_rect(const ui_page_t *page, const ui_tile_t *tile, ui_rect_t area, int16_t gap);

/* The tile with this id on this page, or NULL. */
const ui_tile_t *ui_page_find_tile(const ui_page_t *page, const char *id);

typedef struct {
    void (*bluetooth)(void *ctx, const td_action_t *action);
    void (*key)(void *ctx, uint8_t usage, uint8_t mods);
    void (*mod)(void *ctx, key_mod_t mod);
    void (*consumer)(void *ctx, uint16_t usage);
    void (*page)(void *ctx, uint8_t index);
    void (*press)(void *ctx, const ui_tile_t *tile);
} ui_action_ops_t;

/* Emit a shortcut or media press and release without sharing Companion modifier latches. */
void ui_action_bluetooth(const td_action_t *action, key_sink_t sink);

/*
 * Runs the tile's action through ops. True when it called one. A key, mod, consumer or page action calls its own op.
 * An app or widget action calls `press`, because the deck sends nothing for it. Does nothing for a NULL tile or ops,
 * a NONE action, or an action whose op is NULL. The caller decides that the press was a tap.
 */
bool ui_tile_click(const ui_tile_t *tile, const ui_action_ops_t *ops, void *ctx);

#ifdef __cplusplus
}
#endif
