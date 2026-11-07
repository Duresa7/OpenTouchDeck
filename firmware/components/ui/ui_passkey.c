/*
 * The Bluetooth pairing passkey on screen (radio build). td_ble asks for it from its own tasks with its lock held, so
 * ui_passkey_set only stores what to show; an LVGL timer draws it on the top layer, over the Page and the idle face,
 * and takes it down. Nothing here takes a lock, so td_ble and the display never wait on each other.
 */
#include "ui.h"
#include <stdatomic.h>
#include <stdio.h>
#include "esp_attr.h"
#include "esp_log.h"
#include "lvgl.h"
#include "sdkconfig.h"
#include "ui_theme.h"

#if CONFIG_TD_RADIO_BUILD

#define TICK_MS 100
#define CODE_SCALE (2 * LV_SCALE_NONE)
#define SHOW_BIT 0x80000000u

static const char *TAG = "ui";

/* What td_ble wants: SHOW_BIT and the code, or 0 for nothing. This state lives in PSRAM with the radio build's other
 * .bss, so the passkey costs no static internal RAM. */
static EXT_RAM_BSS_ATTR _Atomic uint32_t s_want;

static EXT_RAM_BSS_ATTR struct {
    lv_timer_t *timer;
    lv_obj_t *panel;
    lv_obj_t *code;
    uint32_t shown;
} s;

void ui_passkey_set(bool show, uint32_t passkey)
{
    atomic_store(&s_want, show ? (SHOW_BIT | (passkey % 1000000u)) : 0u);
}

static lv_obj_t *label(lv_obj_t *parent, const lv_font_t *font, uint16_t color, const char *text)
{
    lv_obj_t *obj = lv_label_create(parent);
    lv_obj_set_style_text_font(obj, font, 0);
    lv_obj_set_style_text_color(obj, ui_color(color), 0);
    lv_obj_set_style_text_align(obj, LV_TEXT_ALIGN_CENTER, 0);
    lv_label_set_text(obj, text);
    return obj;
}

static void build(void)
{
    s.panel = lv_obj_create(lv_layer_top());
    lv_obj_remove_style_all(s.panel);
    lv_obj_set_size(s.panel, lv_pct(100), lv_pct(100));
    lv_obj_set_style_bg_color(s.panel, ui_color(ui_theme.color.bg), 0);
    lv_obj_set_style_bg_opa(s.panel, LV_OPA_COVER, 0);
    /* Taps land on the panel, not on the Tiles under it. */
    lv_obj_set_clickable(s.panel, true);
    lv_obj_set_scrollable(s.panel, false);
    lv_obj_set_flex_flow(s.panel, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_flex_align(s.panel, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_row(s.panel, 24, 0);
    label(s.panel, ui_theme.font_title, ui_theme.color.ink, "Bluetooth pairing");
    /* The widget value font, drawn twice its size like the idle clock, so the code reads from arm's length. */
    s.code = label(s.panel, ui_theme.font_big, ui_theme.color.accent, "000 000");
    lv_obj_set_style_text_letter_space(s.code, 4, 0);
    lv_obj_set_style_transform_scale(s.code, CODE_SCALE, 0);
    lv_obj_set_style_transform_pivot_x(s.code, lv_pct(50), 0);
    lv_obj_set_style_transform_pivot_y(s.code, lv_pct(50), 0);
    /* The scale doesn't take layout space: pad the rows so the title and the hint clear the drawn code. */
    lv_obj_set_style_margin_ver(s.code, 40, 0);
    label(s.panel, ui_theme.font_label, ui_theme.color.ink_2, "Type this code on the other device");
}

static void tick(lv_timer_t *timer)
{
    (void)timer;
    const uint32_t want = atomic_load(&s_want);
    if (want == s.shown) {
        return;
    }
    s.shown = want;
    if (!(want & SHOW_BIT)) {
        if (s.panel != NULL) {
            lv_obj_delete(s.panel);
            s.panel = NULL;
            s.code = NULL;
        }
        ESP_LOGI(TAG, "passkey down");
        return;
    }
    if (s.panel == NULL) {
        build();
    }
    const uint32_t code = (want & ~SHOW_BIT) % 1000000u;
    char text[16];
    snprintf(text, sizeof text, "%03u %03u", (unsigned)(code / 1000), (unsigned)(code % 1000));
    lv_label_set_text(s.code, text);
    ESP_LOGI(TAG, "passkey up");
}

void ui_passkey_start(void)
{
    if (s.timer == NULL) {
        s.timer = lv_timer_create(tick, TICK_MS, NULL);
    }
}

#else

void ui_passkey_set(bool show, uint32_t passkey)
{
    (void)show;
    (void)passkey;
}

void ui_passkey_start(void)
{
}

#endif
