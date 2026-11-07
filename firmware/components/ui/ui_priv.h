#pragma once

#include <stdbool.h>
#include <stdint.h>
#include "lvgl.h"
#include "sdkconfig.h"
#include "ui.h"
#include "td_layout.h"
#include "ui_pages.h"

#define UI_PAGES_MAX 8

#if CONFIG_TD_UI_SKIP_NOOP_SETS
#include "ui_redraw.h"

/*
 * The widget and dashboard redraws set every label of an entry each time an app sends it, and an unchanged set still
 * invalidates the label, which in TRIPLE_PARTIAL costs a whole frame. These skip a set only when the object already
 * holds exactly what the set would store, so it ends as the set would leave it. Bar widths and hidden flags need no
 * check: LVGL 9.6's lv_obj_set_width and lv_obj_set_hidden already return early when nothing changes.
 */
static inline void ui_set_text_if_changed(lv_obj_t *label, const char *text)
{
    /* A dots-mode label that was cut is always set again, as without this option. */
    const char *shown = lv_label_get_text(label);
    const bool cut = lv_label_get_long_mode(label) == LV_LABEL_LONG_MODE_DOTS && ui_text_cut_with_dots(shown);
    if (cut || !ui_text_unchanged(shown, text)) {
        lv_label_set_text(label, text);
    }
}

/* Compares the label's own local style, which is all lv_obj_set_style_text_opa(obj, opa, 0) writes, rather than the
 * resolved opacity, so a style or parent that someday sets text opacity cannot make a needed set look redundant. */
static inline void ui_set_text_opa_if_changed(lv_obj_t *obj, lv_opa_t opa)
{
    lv_style_value_t v;
    if (lv_obj_get_local_style_prop(obj, LV_STYLE_TEXT_OPA, &v, 0) != LV_STYLE_RES_FOUND || v.num != opa) {
        lv_obj_set_style_text_opa(obj, opa, 0);
    }
}
#endif

/*
 * What the console reports about the deck. Totals count the page changes that ran their whole slide, so
 * transition_frames * 1000 / transition_ms is the frame rate during a transition. A slide that a new touch cut
 * short and a bounce at an end count in neither the totals nor the `last_` fields. The strings are copies, so a
 * reader keeps them after the layout changes.
 */
typedef struct {
    uint8_t page;
    uint8_t pages;
    uint8_t tile_count;
    char title[TD_LAYOUT_TITLE_MAX];
    char pressed_tile[TD_LAYOUT_ID_MAX];
    uint32_t taps;
    bool animating;
    uint32_t transitions;
    uint32_t transition_ms;
    uint32_t transition_frames;
    uint32_t last_ms;
    uint32_t last_frames;
    bool online;
    uint16_t dimmed;
} ui_state_t;

/* One tile, copied out of the layout. `icon` is empty for a tile with no icon. */
typedef struct {
    uint8_t page;
    ui_rect_t rect;
    char label[TD_LAYOUT_LABEL_MAX];
    char action[12];
    char icon[24];
} ui_tile_info_t;

/* Copies the deck's state under the layout lock. Valid once ui_ready() is true. The totals are whole when
 * `animating` is false. */
void ui_get_state(ui_state_t *out);

/* Whether the Layout on screen has a title strip, which a full-screen Layout gives to its Tiles. On the LVGL task. */
bool ui_deck_title_shown(void);

/* Slides to a page, as a swipe would. False when the page does not exist. The caller holds the LVGL lock. */
bool ui_goto(uint8_t page);

/* Copies a tile and where it sits on screen while its page shows, under the layout lock. */
bool ui_tile_info(const char *id, ui_tile_info_t *out);

/* The `layout` console command. */
int ui_layout_cmd(int argc, char **argv);

/* The icon cache. A file icon's bitmap stays in PSRAM, keyed by its id, so a reload of the same layout reads nothing
 * again. */
typedef struct {
    uint16_t loaded;
    uint16_t missing;
} ui_icon_counts_t;

/* Reads the `file:` icons of `layout` into the cache and counts each tile's icon as loaded or missing. It only evicts
 * icons that neither `layout` nor `keep` names, so the screen never loses an image it draws. */
ui_icon_counts_t ui_icon_prepare(const td_layout_t *layout, const td_layout_t *keep);

/* The cached image of a `file:` icon, or NULL. */
const lv_image_dsc_t *ui_icon_image(const char *icon);

/* The LVGL symbol text of an `lv:` icon, or NULL. */
const char *ui_icon_symbol(const char *icon);

/*
 * Widget tiles, dashboard pages and the offline dimming. The state table lives behind its own lock, which is a leaf:
 * nothing else is taken while it is held. The objects belong to the LVGL task. ui_widget_begin runs before a layout's
 * objects are built, with the LVGL lock held, and empties the table and the object lists.
 */
void ui_widget_begin(void);

/* Allocates and binds the live values of a Layout being activated, which clears every value and counter. Runs where
 * ui_widget_begin does, before the Layout's objects are built and before the old Layout is freed. */
void ui_live_begin(const td_layout_t *layout);

/* Draws a graph, bar or now-playing Tile on `obj` and ties it to its slots. */
void ui_live_build(lv_obj_t *obj, const ui_tile_t *tile, ui_rect_t rect);

/* Gives back the pins of the covers the last ui_live_begin dropped. Call it once the LVGL lock is released, before the
 * link handles its next message, so nothing takes the storage lock under the LVGL lock. */
void ui_live_unpin_dropped(void);

/* A press at Tile point (x, y) on a bar Tile. True when it lands in the value zone of a fresh bar that takes input:
 * the bar holds the finger from then on, and the pager must not see it. */
bool ui_live_capture(const ui_tile_t *tile, int16_t x, int16_t y);

/* The held finger is at Tile point (x, y). Once it passes the slop the bar draws its value and sends it as a move. */
void ui_live_drag(int16_t x, int16_t y);

/* The held finger lifted or the press was lost. A drag sends its end and settles; a press that never moved sends
 * nothing. */
void ui_live_drop(void);

/* A tap at Tile point (x, y) on a bar or now-playing Tile of Page `page`. When it names a part, from the shared vectors,
 * the part hook gets it with the live-table generation it was named in, and the call returns true. A Tile not redrawn
 * since the last session or Layout change names none. */
bool ui_live_tap(const char *page, const ui_tile_t *tile, int16_t x, int16_t y);

/* Called without the LVGL lock. Decodes the next Layout's posters while the shown Layout keeps playing. */
void ui_gif_prepare(const td_layout_t *layout);
void ui_gif_discard(void);
/* Called with the LVGL lock before the old Tiles are deleted. Waits for the worker to release the
 * outgoing Layout, then installs the prepared one. */
void ui_gif_begin(void);
bool ui_gif_build(lv_obj_t *obj, const ui_tile_t *tile, uint8_t page);
void ui_gif_page(uint8_t page, bool sliding);
/* Any task may call it. The LVGL timer applies it. */
void ui_gif_set_dimmed(bool dimmed);
void ui_gif_resume(void);

/* Draws a widget tile's value, caption and bar on its tile object and registers the tile in the table. */
void ui_widget_build(lv_obj_t *obj, const ui_tile_t *tile, ui_rect_t rect);

/* Registers an app or widget tile so it dims while the app is offline. Does nothing for other tiles. */
void ui_widget_track(lv_obj_t *obj, const ui_tile_t *tile);

/* True when a tap on this tile must send nothing because the tile is dimmed. */
bool ui_widget_tap_blocked(const ui_tile_t *tile);

/* How many tiles are dimmed now. */
uint16_t ui_widget_dimmed(void);

/* Copies an entry of the table. False for an id the layout does not have. */
bool ui_widget_read(const char *tile, ui_widget_state_t *out);
bool ui_dashboard_read(const char *page, ui_dash_state_t *out);

/* Registers a dashboard page in the table, under the table lock. False when the table is full. */
bool ui_widget_add_page(const char *page);

/* The dashboard drawing, in ui_dashboard.c. begin forgets the old rows, build draws a page's empty rows and registers
 * the page, and draw shows the state of the page at table index `index`. */
void ui_dashboard_begin(void);
void ui_dashboard_build(lv_obj_t *page_obj, const ui_page_t *page);
void ui_dashboard_draw(int index, const ui_dash_state_t *state);
