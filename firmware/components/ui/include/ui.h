#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "esp_console.h"
#include "key_sink.h"
#include "td_layout.h"
#include "ui_widget_state.h"
#include "ui_live.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Reads /storage/layouts/<orientation>.json for the first build. A missing or invalid file logs
 * `layout: <reason>, using builtin`, and the deck starts on the built-in layout. Call it before display_start(). */
void ui_layout_boot(const char *orientation);

/* Builds the deck on LVGL's active screen from the boot layout: the pages, their tiles and the page dots. It is the
 * build callback for display_start(), so it runs once with the LVGL adapter lock held, before the first frame. A
 * `layout reload` later rebuilds the same objects from another layout. */
void ui_build(void);

#define UI_LAYOUT_ERROR_MAX 196

/* The layout on screen. `error` is the latest refusal, of the boot file or of a candidate a reload kept off the
 * screen, and is empty after a load that worked. */
typedef struct {
    bool from_storage;
    char name[TD_LAYOUT_TITLE_MAX];
    char orientation[12];
    bool full_screen;
    uint8_t pages;
    uint16_t tiles;
    uint16_t icons_loaded;
    uint16_t icons_missing;
    uint32_t bytes;
    uint32_t load_ms;
    char error[UI_LAYOUT_ERROR_MAX];
} ui_layout_info_t;

/* Copies what `layout state` reports, under the layout lock. */
void ui_layout_info(ui_layout_info_t *out);

/* Loads the stored layout for the boot orientation, or the built-in one with `use_builtin`, and swaps it onto the
 * screen. A candidate that fails leaves the current layout on screen and its error in `layout state`. Takes the
 * LVGL lock, so never call it from an LVGL callback. On failure *err says why. */
bool ui_layout_reload(bool use_builtin, td_layout_error_t *err);

/* True once ui_build() has drawn a deck. */
bool ui_ready(void);

/* The index of the Page on screen, and in *layout a count that changes with every Layout load. Takes no lock. */
uint8_t ui_page_shown(uint32_t *layout);

/* Copies the id of Page `index` of the Layout on screen, under the layout lock. False when it has no such Page. */
bool ui_page_id(uint8_t index, char *out, size_t size);

/* The orientation the deck booted in, which names the layout file a put replaces and a reload reads. */
const char *ui_layout_slot(void);

/* Leaves recovery mode for a deliberate act by the app or a lane, named by `why`, and takes the `Recovery` caption
 * off the screen. A no-op outside recovery mode. Takes the LVGL lock, so never call it from an LVGL callback. While
 * the deck is in recovery mode a reload of the stored layout reloads the built-in one, so call this first. */
void ui_leave_recovery(const char *why);

/* Called on the LVGL task for every tap on an app or widget tile: the page id, the tile id, and the kind, which is
 * "launch", "open", "command" or "widget". The hook must not block. */
typedef void (*ui_press_hook_t)(const char *page, const char *tile, const char *kind);
void ui_set_press_hook(ui_press_hook_t hook);

/* Called on the LVGL task instead of the press hook for a tap on a part of a bar or now-playing Tile, with the part's
 * wire name: "previous", "play_pause", "next" or "mute", and the live-table generation the part was named in. The
 * tap must not go out once ui_live_generation() has left that generation. The hook must not block. */
typedef void (*ui_part_hook_t)(const char *page, const char *tile, const char *kind, const char *part,
                               uint32_t generation);
void ui_set_part_hook(ui_part_hook_t hook);

/* Called on the LVGL task while a finger drags a bar Tile that takes input: `input` with the value it sets, 0 to 1,
 * on each move past the slop, then once with `end` when it lifts or the press is lost. `cancel` drops what has not
 * gone out, on a Layout or session change or when the value goes stale. Neither may block. */
void ui_set_input_hooks(void (*input)(const char *page, const char *tile, double value, bool end),
                        void (*cancel)(void));

/* Tells the deck how to ask whether the app is connected. Until a source is set the deck counts as offline. While
 * it is offline, every app and widget tile draws at the theme's dim_opa and a tap on one sends nothing. The deck asks
 * on its widget redraw timer, so a change shows within one redraw. */
void ui_set_link_online(bool (*online)(void));

/* Applies one `v2.live.set` to the Layout on screen, under the live-value lock, and returns why it was refused or
 * UI_LIVE_OK. *retry_ms is set for UI_LIVE_RATE. */
ui_live_result_t ui_live_update(const ui_live_set_t *set, uint32_t *retry_ms);

/* A new protocol 2 selection: clears every live value and counter, restarts the rate windows, ends the Live tile
 * touch the old session held and drops its queued input through the input hooks. */
void ui_live_select(void);

/* The live-table generation now. Every Layout and session change advances it. */
uint32_t ui_live_generation(void);

/* True when the online source says the app is connected. */
bool ui_link_online(void);

/* The idle face: the photo frame's picture, or the clock, over the Page while power_idle_face() says so. ui_build
 * starts the timer that raises it, takes it down and keeps it on top. */
void ui_idle_start(void);

/* The Bluetooth pairing passkey (radio build): show true puts the six-digit code on the top layer, over the Page and
 * the idle face, and false takes it down. Any task, without a lock, and it never blocks; a timer that ui_build starts
 * draws it within 100 ms. The normal build draws nothing. */
void ui_passkey_set(bool show, uint32_t passkey);
void ui_passkey_start(void);

typedef enum {
    UI_PICTURE_SHOWN,
    UI_PICTURE_FORMAT, /* not a baseline colour JPEG */
    UI_PICTURE_SIZE,   /* larger than the screen */
    UI_PICTURE_IDLE,   /* the idle face went down, or the display stayed busy */
    UI_PICTURE_MEMORY,
} ui_picture_t;

/* Decodes one JPEG on the caller's task and shows it on the idle face in place of the last picture, which is freed. The
 * face keeps only the picture it shows, and drops it when it goes down. Takes the LVGL lock, so never call it from an
 * LVGL callback. */
ui_picture_t ui_idle_picture(const uint8_t *jpeg, size_t len);

/* Sets the clock to unix_s in UTC and offset_min east of it, counting on from now. Any task. */
void ui_idle_set_clock(uint64_t unix_s, int32_t offset_min);

/* After two intervals without a picture the face shows the clock. Any task. */
void ui_idle_set_interval(uint32_t interval_s);

/* Writes a widget tile's state and marks it for the next redraw, from any task and without the LVGL lock. Returns
 * NULL and the stored state in *stored (when not NULL), or the refusal: "unknown tile", "display not ready". */
const char *ui_widget_update(const char *tile, const char *value, const char *caption, float bar, bool stale,
                             ui_widget_state_t *stored);

/* Replaces a dashboard page's rows the same way. Refusals: "unknown page", "too many rows", "display not ready". */
const char *ui_dashboard_update(const char *page, const ui_dash_row_t *rows, size_t count);

/* The ui console commands, for td_console_register(). Before the display has started, a well-formed command
 * answers "display not ready". */
const esp_console_cmd_t *ui_console_commands(size_t *count);

/* Radio build only: the net-v2 pairing screen, full screen over everything with a Cancel button that calls on_cancel
 * on the LVGL task. With code NULL the window waits for an app; with a code it shows the six digits and the app's
 * name. Call it again to update the time left; ui_pairing_hide takes it down. Any task but the LVGL task. */
void ui_pairing_show(const char *code, const char *name, uint32_t left_s, void (*on_cancel)(void));
void ui_pairing_hide(void);

/* Configure the Tile Bluetooth destination before UI start. */
void ui_set_bluetooth_sink(key_sink_t sink);

#ifdef __cplusplus
}
#endif
