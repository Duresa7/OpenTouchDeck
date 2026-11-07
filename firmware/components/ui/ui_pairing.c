#include "sdkconfig.h"

/* The net-v2 pairing screen. Only the radio build pairs, so the normal build compiles none of this. */
#if CONFIG_TD_RADIO_BUILD

#include "ui.h"
#include "ui_priv.h"
#include "ui_theme.h"
#include <stdio.h>
#include <string.h>
#include "esp_log.h"
#include "esp_lv_adapter.h"
#include "lvgl.h"

static const char *TAG = "ui";

#define LOCK_TIMEOUT_MS 3000
#define CANCEL_W 280
#define CANCEL_H 96

/* The code lives here only while the screen shows it. Every field is touched under the adapter lock. */
static struct {
    lv_obj_t *overlay;
    lv_obj_t *heading;
    lv_obj_t *code;
    lv_obj_t *left;
    void (*on_cancel)(void);
} s;

static void on_cancel_click(lv_event_t *e)
{
    (void)e;
    ESP_LOGI(TAG, "pairing cancel");
    if (s.on_cancel) {
        s.on_cancel();
    }
}

static lv_obj_t *add_label(lv_obj_t *parent, const lv_font_t *font, uint16_t color, int32_t y)
{
    lv_obj_t *label = lv_label_create(parent);
    lv_label_set_text_static(label, "");
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, ui_color(color), 0);
    lv_obj_set_style_text_align(label, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_width(label, lv_pct(90));
    lv_obj_align(label, LV_ALIGN_CENTER, 0, y);
    return label;
}

/* Covers every screen from LVGL's top layer, so a Layout reload or a page swipe can't hide it, and it takes every
 * touch. */
static void build(void)
{
    lv_obj_t *overlay = lv_obj_create(lv_layer_top());
    lv_obj_remove_style_all(overlay);
    lv_obj_set_scrollable(overlay, false);
    lv_obj_set_clickable(overlay, true);
    lv_obj_set_size(overlay, lv_pct(100), lv_pct(100));
    lv_obj_set_style_bg_color(overlay, ui_color(ui_theme.color.bg), 0);
    lv_obj_set_style_bg_opa(overlay, LV_OPA_COVER, 0);
    s.overlay = overlay;

    lv_obj_t *title = add_label(overlay, ui_theme.font_title, ui_theme.color.ink, -200);
    lv_label_set_text_static(title, "Pair a new app");
    s.heading = add_label(overlay, ui_theme.font_label, ui_theme.color.ink_2, -130);
    s.code = add_label(overlay, ui_theme.font_big, ui_theme.color.ink, -40);
    lv_obj_set_style_text_letter_space(s.code, 18, 0);
    s.left = add_label(overlay, ui_theme.font_label, ui_theme.color.ink_2, 40);

    lv_obj_t *cancel = lv_button_create(overlay);
    lv_obj_remove_style_all(cancel);
    lv_obj_set_size(cancel, CANCEL_W, CANCEL_H);
    lv_obj_align(cancel, LV_ALIGN_CENTER, 0, 150);
    lv_obj_set_style_bg_color(cancel, ui_color(ui_theme.color.tile), 0);
    lv_obj_set_style_bg_color(cancel, ui_color(ui_theme.color.tile_raised), LV_STATE_PRESSED);
    lv_obj_set_style_bg_opa(cancel, LV_OPA_COVER, 0);
    lv_obj_set_style_radius(cancel, ui_theme.tile_radius, 0);
    lv_obj_add_event_cb(cancel, on_cancel_click, LV_EVENT_CLICKED, NULL);
    lv_obj_t *label = lv_label_create(cancel);
    lv_label_set_text_static(label, "Cancel");
    lv_obj_set_style_text_font(label, ui_theme.font_label, 0);
    lv_obj_set_style_text_color(label, ui_color(ui_theme.color.ink), 0);
    lv_obj_center(label);
}

void ui_pairing_show(const char *code, const char *name, uint32_t left_s, void (*on_cancel)(void))
{
    if (!ui_ready()) {
        return;
    }
    if (esp_lv_adapter_lock(LOCK_TIMEOUT_MS) != ESP_OK) {
        ESP_LOGW(TAG, "pairing screen not drawn: display busy");
        return;
    }
    if (!s.overlay) {
        build();
    }
    s.on_cancel = on_cancel;
    char text[96];
    if (code) {
        snprintf(text, sizeof text, "Type this code in %s", name && *name ? name : "the app");
        lv_label_set_text(s.heading, text);
        snprintf(text, sizeof text, "%.3s %.3s", code, code + 3);
        lv_label_set_text(s.code, text);
    } else {
        lv_label_set_text_static(s.heading, "Choose this Device in the app");
        lv_label_set_text_static(s.code, "");
    }
    snprintf(text, sizeof text, "%u:%02u left", (unsigned)(left_s / 60), (unsigned)(left_s % 60));
    lv_label_set_text(s.left, text);
    /* The code is gone from the text buffer once LVGL copied it. */
    memset(text, 0, sizeof text);
    esp_lv_adapter_unlock();
}

void ui_pairing_hide(void)
{
    if (!ui_ready() || esp_lv_adapter_lock(LOCK_TIMEOUT_MS) != ESP_OK) {
        return;
    }
    if (s.overlay) {
        lv_obj_delete(s.overlay);
    }
    memset(&s, 0, sizeof s);
    esp_lv_adapter_unlock();
}

#endif
