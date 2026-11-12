/*
 * Settings on the Device (radio build): a gear at the right end of the title strip opens a full-screen panel over the
 * Page with the paired apps, net-v2's Pairings (schema/protocol/net-v2.md, "Pairing slots"). It lists each app's name,
 * forgets one or all of them after a confirmation on the screen, and opens a pairing window with Pair new app. A full
 * Device opens no window and says how to free a slot.
 *
 * The net-v2 service owns the Pairings and hands over hooks (ui_settings_set_hooks). The panel reads them on a timer
 * and asks for changes without waiting, so nothing here blocks the LVGL task. The normal build has no hooks, so it
 * draws no gear, and compiles none of this.
 */
#include "ui.h"
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include "sdkconfig.h"

#if CONFIG_TD_RADIO_BUILD

#include "esp_attr.h"
#include "esp_log.h"
#include "lvgl.h"
#include "power.h"
#include "td_recovery.h"
#include "ui_priv.h"
#include "ui_theme.h"

static const char *TAG = "ui";

#define TICK_MS 250
#define GEAR_BOX 64
#define PAD 32
#define ROW_H 88
#define BUTTON_H 80
#define BUTTON_W 240
#define FORGET_W 160
#define CONFIRM_W 640

static _Atomic(const ui_settings_hooks_t *) s_hooks;

/* The panel and the copy it shows. It lives in PSRAM with the radio build's other .bss, so it costs no static internal
 * RAM, and the panel's objects exist only while it is open. Every field is touched on the LVGL task. */
static EXT_RAM_BSS_ATTR struct {
    lv_timer_t *timer;
    lv_obj_t *gear;
    lv_obj_t *panel;
    lv_obj_t *count;
    lv_obj_t *list;
    lv_obj_t *message;
    lv_obj_t *pair;
    lv_obj_t *forget_all;
    lv_obj_t *confirm;
    ui_paired_apps_t shown;
    /* The row a confirmation is for, or -1 for every app. */
    int confirming;
} s;

static lv_obj_t *add_label(lv_obj_t *parent, const lv_font_t *font, uint16_t color, const char *text)
{
    lv_obj_t *label = lv_label_create(parent);
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, ui_color(color), 0);
    lv_label_set_text(label, text);
    return label;
}

static lv_obj_t *add_button(lv_obj_t *parent, const char *text, uint16_t fill, uint16_t ink, int32_t w,
                            lv_event_cb_t on_click, void *user_data)
{
    lv_obj_t *button = lv_button_create(parent);
    lv_obj_remove_style_all(button);
    lv_obj_set_size(button, w, BUTTON_H);
    lv_obj_set_style_bg_color(button, ui_color(fill), 0);
    lv_obj_set_style_bg_color(button, ui_color(ui_theme.color.tile_raised), LV_STATE_PRESSED);
    lv_obj_set_style_bg_opa(button, LV_OPA_COVER, 0);
    lv_obj_set_style_radius(button, ui_theme.tile_radius, 0);
    lv_obj_add_event_cb(button, on_click, LV_EVENT_CLICKED, user_data);
    lv_obj_t *label = add_label(button, ui_theme.font_label, ink, text);
    lv_obj_center(label);
    return button;
}

static void set_enabled(lv_obj_t *button, bool enabled)
{
    if (enabled) {
        lv_obj_remove_state(button, LV_STATE_DISABLED);
        lv_obj_set_style_opa(button, LV_OPA_COVER, 0);
    } else {
        lv_obj_add_state(button, LV_STATE_DISABLED);
        lv_obj_set_style_opa(button, LV_OPA_50, 0);
    }
}

/* Both may run in a click on one of their own buttons, so LVGL deletes them after the event. */
static void close_confirm(void)
{
    if (s.confirm) {
        lv_obj_delete_async(s.confirm);
        s.confirm = NULL;
    }
}

static void close_panel(void)
{
    close_confirm();
    if (s.panel) {
        lv_obj_delete_async(s.panel);
    }
    s.panel = s.count = s.list = s.message = s.pair = s.forget_all = NULL;
    ESP_LOGI(TAG, "settings closed");
}

static const ui_settings_hooks_t *hooks(void)
{
    return atomic_load(&s_hooks);
}

/* ---- The confirmation ---- */

static void on_confirm_cancel(lv_event_t *e)
{
    (void)e;
    close_confirm();
}

static void on_confirm_forget(lv_event_t *e)
{
    (void)e;
    const ui_settings_hooks_t *h = hooks();
    const int row = s.confirming;
    close_confirm();
    if (!h || s.shown.busy) {
        return;
    }
    if (row < 0) {
        ESP_LOGI(TAG, "settings: forget all apps");
        h->forget_all();
    } else if (row < s.shown.count) {
        ESP_LOGI(TAG, "settings: forget one app");
        h->forget(s.shown.apps[row].app_id);
    }
}

/* A dialog over the panel, on the top layer above it. Nothing is erased until the person taps its Forget button. */
static void open_confirm(int row)
{
    close_confirm();
    s.confirming = row;
    lv_obj_t *scrim = lv_obj_create(lv_layer_top());
    lv_obj_remove_style_all(scrim);
    lv_obj_set_size(scrim, lv_pct(100), lv_pct(100));
    lv_obj_set_style_bg_color(scrim, ui_color(ui_theme.color.bg), 0);
    lv_obj_set_style_bg_opa(scrim, LV_OPA_80, 0);
    lv_obj_set_clickable(scrim, true);
    lv_obj_set_scrollable(scrim, false);
    s.confirm = scrim;

    lv_obj_t *box = lv_obj_create(scrim);
    lv_obj_remove_style_all(box);
    lv_obj_set_size(box, LV_MIN(CONFIRM_W, lv_display_get_horizontal_resolution(NULL) - 2 * PAD), LV_SIZE_CONTENT);
    lv_obj_center(box);
    lv_obj_set_style_bg_color(box, ui_color(ui_theme.color.tile), 0);
    lv_obj_set_style_bg_opa(box, LV_OPA_COVER, 0);
    lv_obj_set_style_radius(box, ui_theme.tile_radius, 0);
    lv_obj_set_style_pad_all(box, PAD, 0);
    lv_obj_set_style_pad_row(box, 20, 0);
    lv_obj_set_flex_flow(box, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_scrollable(box, false);

    char text[UI_PAIRED_APP_NAME_MAX + 32];
    if (row < 0) {
        snprintf(text, sizeof text, "Forget every paired app?");
    } else {
        snprintf(text, sizeof text, "Forget %s?", s.shown.apps[row].name);
    }
    lv_obj_t *title = add_label(box, ui_theme.font_label, ui_theme.color.ink, text);
    lv_obj_set_width(title, lv_pct(100));
    lv_label_set_long_mode(title, LV_LABEL_LONG_MODE_WRAP);
    lv_obj_t *hint = add_label(box, ui_theme.font_sub, ui_theme.color.ink_2,
                               row < 0 ? "None of them can reach this Device over Wi-Fi until you pair it again."
                                       : "It can't reach this Device over Wi-Fi until you pair it again.");
    lv_obj_set_width(hint, lv_pct(100));
    lv_label_set_long_mode(hint, LV_LABEL_LONG_MODE_WRAP);

    lv_obj_t *buttons = lv_obj_create(box);
    lv_obj_remove_style_all(buttons);
    lv_obj_set_size(buttons, lv_pct(100), LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(buttons, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(buttons, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_column(buttons, 20, 0);
    add_button(buttons, "Cancel", ui_theme.color.tile_raised, ui_theme.color.ink, FORGET_W, on_confirm_cancel, NULL);
    add_button(buttons, row < 0 ? "Forget all" : "Forget", UI_SCREEN_DANGER, ui_theme.color.bg, FORGET_W,
               on_confirm_forget, NULL);
}

/* ---- The panel ---- */

static void on_forget(lv_event_t *e)
{
    const int row = (int)(intptr_t)lv_event_get_user_data(e);
    if (!s.shown.busy && row < s.shown.count) {
        open_confirm(row);
    }
}

static void on_forget_all(lv_event_t *e)
{
    (void)e;
    if (!s.shown.busy && s.shown.count > 0) {
        open_confirm(-1);
    }
}

static void on_pair(lv_event_t *e)
{
    (void)e;
    const ui_settings_hooks_t *h = hooks();
    if (!h || s.shown.busy || s.shown.pairing || s.shown.locked_s > 0) {
        return;
    }
    /* A full Device opens no window. The panel already says how to free a slot, above the Forget buttons. */
    if (s.shown.count >= UI_PAIRED_APPS_MAX) {
        ESP_LOGI(TAG, "settings: pair new app refused, four apps are paired");
        return;
    }
    ESP_LOGI(TAG, "settings: pair new app");
    h->pair();
}

static void on_done(lv_event_t *e)
{
    (void)e;
    close_panel();
}

static void fill_list(void)
{
    lv_obj_clean(s.list);
    if (s.shown.count == 0) {
        lv_obj_t *empty = add_label(s.list, ui_theme.font_sub, ui_theme.color.ink_2,
                                    "No apps are paired. Choose Pair new app, then type the code this Device shows "
                                    "into the app.");
        lv_obj_set_width(empty, lv_pct(100));
        lv_label_set_long_mode(empty, LV_LABEL_LONG_MODE_WRAP);
        return;
    }
    for (uint8_t i = 0; i < s.shown.count; i++) {
        lv_obj_t *row = lv_obj_create(s.list);
        lv_obj_remove_style_all(row);
        lv_obj_set_size(row, lv_pct(100), ROW_H);
        lv_obj_set_style_bg_color(row, ui_color(ui_theme.color.tile), 0);
        lv_obj_set_style_bg_opa(row, LV_OPA_COVER, 0);
        lv_obj_set_style_radius(row, ui_theme.tile_radius, 0);
        lv_obj_set_style_pad_hor(row, 24, 0);
        lv_obj_set_flex_flow(row, LV_FLEX_FLOW_ROW);
        lv_obj_set_flex_align(row, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
        lv_obj_set_scrollable(row, false);
        lv_obj_t *name = add_label(row, ui_theme.font_label, ui_theme.color.ink, s.shown.apps[i].name);
        lv_label_set_long_mode(name, LV_LABEL_LONG_MODE_DOTS);
        lv_obj_set_flex_grow(name, 1);
        lv_obj_t *forget = add_button(row, "Forget", ui_theme.color.tile_raised, ui_theme.color.ink, FORGET_W,
                                      on_forget, (void *)(intptr_t)i);
        lv_obj_set_height(forget, ROW_H - 24);
    }
}

static void draw(void)
{
    char text[64];
    snprintf(text, sizeof text, "%u of %u", (unsigned)s.shown.count, (unsigned)UI_PAIRED_APPS_MAX);
    lv_label_set_text(s.count, text);
    fill_list();

    const char *message = NULL;
    uint16_t color = ui_theme.color.signal;
    if (s.shown.busy) {
        message = "Saving...";
        color = ui_theme.color.ink_2;
    } else if (s.shown.problem) {
        message = s.shown.problem;
    } else if (s.shown.locked_s > 0) {
        snprintf(text, sizeof text, "Too many wrong codes. Pair again in %u:%02u.", (unsigned)(s.shown.locked_s / 60),
                 (unsigned)(s.shown.locked_s % 60));
        message = text;
    } else if (s.shown.count >= UI_PAIRED_APPS_MAX) {
        message = UI_PAIRED_APPS_FULL;
    }
    lv_label_set_text(s.message, message ? message : "");
    lv_obj_set_style_text_color(s.message, ui_color(color), 0);
    set_enabled(s.pair, !s.shown.busy && !s.shown.pairing && s.shown.locked_s == 0);
    set_enabled(s.forget_all, !s.shown.busy && s.shown.count > 0);
    lv_obj_set_hidden(s.forget_all, s.shown.count == 0);
}

static void open_panel(void)
{
    const ui_settings_hooks_t *h = hooks();
    if (!h || s.panel) {
        return;
    }
    lv_obj_t *panel = lv_obj_create(lv_layer_top());
    lv_obj_remove_style_all(panel);
    lv_obj_set_size(panel, lv_pct(100), lv_pct(100));
    lv_obj_set_style_bg_color(panel, ui_color(ui_theme.color.bg), 0);
    lv_obj_set_style_bg_opa(panel, LV_OPA_COVER, 0);
    lv_obj_set_style_pad_all(panel, PAD, 0);
    lv_obj_set_style_pad_row(panel, 20, 0);
    lv_obj_set_flex_flow(panel, LV_FLEX_FLOW_COLUMN);
    /* Taps land on the panel, not on the Tiles under it. */
    lv_obj_set_clickable(panel, true);
    lv_obj_set_scrollable(panel, false);
    s.panel = panel;

    lv_obj_t *header = lv_obj_create(panel);
    lv_obj_remove_style_all(header);
    lv_obj_set_size(header, lv_pct(100), LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(header, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(header, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    add_label(header, ui_theme.font_big, ui_theme.color.ink, "Settings");
    add_button(header, "Done", ui_theme.color.tile, ui_theme.color.ink, FORGET_W, on_done, NULL);

    lv_obj_t *section = lv_obj_create(panel);
    lv_obj_remove_style_all(section);
    lv_obj_set_size(section, lv_pct(100), LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(section, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(section, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    add_label(section, ui_theme.font_label, ui_theme.color.ink_2, "Paired apps");
    s.count = add_label(section, ui_theme.font_label, ui_theme.color.ink_2, "");

    s.list = lv_obj_create(panel);
    lv_obj_remove_style_all(s.list);
    lv_obj_set_width(s.list, lv_pct(100));
    lv_obj_set_flex_grow(s.list, 1);
    lv_obj_set_flex_flow(s.list, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_style_pad_row(s.list, 12, 0);
    lv_obj_set_scrollable(s.list, true);
    lv_obj_set_scroll_dir(s.list, LV_DIR_VER);

    s.message = add_label(panel, ui_theme.font_sub, ui_theme.color.signal, "");
    lv_obj_set_width(s.message, lv_pct(100));
    lv_label_set_long_mode(s.message, LV_LABEL_LONG_MODE_WRAP);

    lv_obj_t *buttons = lv_obj_create(panel);
    lv_obj_remove_style_all(buttons);
    lv_obj_set_size(buttons, lv_pct(100), LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(buttons, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(buttons, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    s.pair = add_button(buttons, "Pair new app", ui_theme.color.accent, ui_theme.color.bg, BUTTON_W, on_pair, NULL);
    s.forget_all = add_button(buttons, "Forget all", ui_theme.color.tile, UI_SCREEN_DANGER, BUTTON_W, on_forget_all,
                              NULL);

    h->read(&s.shown);
    draw();
    ESP_LOGI(TAG, "settings open, %u paired", (unsigned)s.shown.count);
}

static void on_gear(lv_event_t *e)
{
    (void)e;
    open_panel();
}

/* ---- The gear ---- */

static void build_gear(void)
{
    const ui_geometry_t *g = ui_geo();
    lv_obj_t *gear = lv_button_create(lv_layer_top());
    lv_obj_remove_style_all(gear);
    lv_obj_set_size(gear, GEAR_BOX, GEAR_BOX);
    /* Its glyph sits where the title strip's right margin ends, centred on the title's line. */
    lv_obj_align(gear, LV_ALIGN_TOP_RIGHT, -(g->title_x - (GEAR_BOX - 32) / 2),
                 g->title_y + (lv_font_get_line_height(ui_theme.font_title) - GEAR_BOX) / 2);
    lv_obj_set_style_radius(gear, GEAR_BOX / 2, 0);
    lv_obj_set_style_bg_color(gear, ui_color(ui_theme.color.tile_raised), LV_STATE_PRESSED);
    lv_obj_set_style_bg_opa(gear, LV_OPA_COVER, LV_STATE_PRESSED);
    lv_obj_add_event_cb(gear, on_gear, LV_EVENT_CLICKED, NULL);
    lv_obj_t *glyph = add_label(gear, ui_theme.font_glyph, ui_theme.color.ink_2, LV_SYMBOL_SETTINGS);
    lv_obj_center(glyph);
    s.gear = gear;
}

/* Runs on the LVGL task with the adapter lock held. */
static void tick(lv_timer_t *timer)
{
    (void)timer;
    const ui_settings_hooks_t *h = hooks();
    if (!h) {
        return;
    }
    if (!s.gear) {
        build_gear();
    }
    /* The gear belongs to the title strip: none on a full-screen Layout, over the idle face or in recovery. */
    lv_obj_set_hidden(s.gear, s.panel != NULL || !ui_deck_title_shown() || power_idle_face(NULL) || td_recovery_mode());
    if (!s.panel) {
        return;
    }
    ui_paired_apps_t now;
    h->read(&now);
    const bool changed = now.version != s.shown.version || now.count != s.shown.count || now.busy != s.shown.busy
                         || now.pairing != s.shown.pairing || now.locked_s != s.shown.locked_s
                         || now.problem != s.shown.problem;
    if (changed) {
        /* A Pairing that went away mid-confirmation may have moved its row: ask again. */
        if (s.confirm && now.version != s.shown.version) {
            close_confirm();
        }
        s.shown = now;
        draw();
    }
}

void ui_settings_set_hooks(const ui_settings_hooks_t *settings_hooks)
{
    atomic_store(&s_hooks, settings_hooks);
}

void ui_settings_start(void)
{
    if (s.timer == NULL) {
        s.confirming = -1;
        s.timer = lv_timer_create(tick, TICK_MS, NULL);
    }
}

#else

void ui_settings_set_hooks(const ui_settings_hooks_t *settings_hooks)
{
    (void)settings_hooks;
}

void ui_settings_start(void)
{
}

#endif
