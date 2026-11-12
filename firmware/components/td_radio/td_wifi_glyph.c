#include "td_wifi_glyph.h"
#include "display.h"
#include "esp_log.h"
#include "esp_lv_adapter.h"
#include "lvgl.h"
#include "td_recovery.h"
#include "td_wifi.h"

static const char *TAG = "td_wifi";

/*
 * A small Wi-Fi glyph at the right end of the title band, on LVGL's top layer, so a Layout reload never removes it and
 * a change redraws only its own box. It shows only while a network is stored: ink-2 when joined, the pager's idle grey
 * while joining, and the signal amber after a failure. The colors are the ui theme's tokens (ui_theme.h).
 */
#define GLYPH_JOINED 0x9098b0
#define GLYPH_JOINING 0x606470
#define GLYPH_FAILED 0xffb020
#define GLYPH_POLL_MS 500
#define LOCK_TIMEOUT_MS 3000
/* The glyph's right inset and top (the band's middle for 32 px). The Settings gear (ui_settings.c) takes the right end
 * of the title band, inside ui_geometry.c's TITLE_X, so the glyph sits 24 px to its left. In recovery mode the
 * `Recovery` caption takes the right end instead, and the glyph sits to its left. */
#define GLYPH_INSET (36 + 32 + 24)
#define GLYPH_TOP 22
#define GLYPH_RECOVERY_INSET 196

static lv_obj_t *s_glyph;
static int s_shown = -1;

static uint32_t color(td_wifi_status_t status)
{
    switch (status) {
    case TD_WIFI_CONNECTED: return GLYPH_JOINED;
    case TD_WIFI_CONNECTING: return GLYPH_JOINING;
    default: return GLYPH_FAILED;
    }
}

/* Runs on the LVGL task with the adapter lock held. */
static void update(lv_timer_t *timer)
{
    (void)timer;
    const td_wifi_status_t status = td_wifi_status();
    if ((int)status == s_shown) {
        return;
    }
    s_shown = (int)status;
    if (status == TD_WIFI_OFF) {
        lv_obj_set_hidden(s_glyph, true);
        return;
    }
    lv_obj_set_style_text_color(s_glyph, lv_color_hex(color(status)), 0);
    lv_obj_set_hidden(s_glyph, false);
}

void td_wifi_glyph_start(void)
{
    if (s_glyph || !display_ready()) {
        return;
    }
    if (esp_lv_adapter_lock(LOCK_TIMEOUT_MS) != ESP_OK) {
        ESP_LOGW(TAG, "indicator not shown: the display is busy");
        return;
    }
    s_glyph = lv_label_create(lv_layer_top());
    lv_label_set_text_static(s_glyph, LV_SYMBOL_WIFI);
    lv_obj_set_style_text_font(s_glyph, &lv_font_montserrat_32, 0);
    lv_obj_align(s_glyph, LV_ALIGN_TOP_RIGHT, td_recovery_mode() ? -GLYPH_RECOVERY_INSET : -GLYPH_INSET, GLYPH_TOP);
    lv_obj_set_clickable(s_glyph, false);
    lv_obj_set_hidden(s_glyph, true);
    update(NULL);
    lv_timer_create(update, GLYPH_POLL_MS, NULL);
    esp_lv_adapter_unlock();
}
