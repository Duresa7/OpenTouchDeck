#include "ui_tile_keys.h"
#include "keys.h"
#include "ui_theme.h"
#include "esp_log.h"
#include "ui_priv.h"

/* The console, a swipe and the idle code write the engine too, so the subjects follow keys_get_state() and not the
 * tiles' own taps. lv_subject_set_int notifies only on change, so a quiet poll redraws nothing. */
static lv_subject_t *s_mod[KEY_MOD_COUNT];
static lv_timer_t *s_poll;

static void sync_mods(void)
{
    const key_engine_state_t st = keys_get_state();
    for (unsigned m = 0; m < KEY_MOD_COUNT; m++) {
        lv_subject_set_int(s_mod[m], st.mods[m].state);
    }
}

static void on_poll(lv_timer_t *timer)
{
    (void)timer;
    sync_mods();
}

static void apply_look(lv_observer_t *observer, lv_subject_t *subject)
{
    lv_obj_t *tile = lv_observer_get_target_obj(observer);
    const int32_t state = lv_subject_get_int(subject);
    lv_obj_set_state(tile, LV_STATE_USER_1, state == KEY_MOD_ONE_SHOT);
    lv_obj_set_state(tile, LV_STATE_CHECKED, state == KEY_MOD_LOCKED);
}

static void apply_glyph(lv_observer_t *observer, lv_subject_t *subject)
{
    lv_obj_set_hidden(lv_observer_get_target_obj(observer), lv_subject_get_int(subject) != KEY_MOD_LOCKED);
}

/* Not clickable, so a tap that lands on the glyph still reaches the tile, and on_press in ui_deck.c finds it. */
static void add_glyph_part(lv_obj_t *tile, lv_subject_t *subject, const lv_style_t *style, int32_t w, int32_t h,
                           int32_t x_ofs, int32_t y_ofs)
{
    lv_obj_t *part = lv_obj_create(tile);
    lv_obj_remove_style_all(part);
    lv_obj_add_style(part, style, 0);
    lv_obj_set_size(part, w, h);
    lv_obj_set_clickable(part, false);
    lv_obj_set_scrollable(part, false);
    lv_obj_align(part, LV_ALIGN_RIGHT_MID, x_ofs, y_ofs);
    lv_subject_add_observer_obj(subject, apply_glyph, part, NULL);
}

void ui_tile_keys_start(void)
{
    if (s_mod[0] == NULL) {
        for (unsigned m = 0; m < KEY_MOD_COUNT; m++) {
            s_mod[m] = lv_subject_create(LV_SUBJECT_TYPE_INT);
        }
    }
    if (s_poll != NULL) {
        lv_timer_delete(s_poll);
    }
    sync_mods();
    s_poll = lv_timer_create(on_poll, ui_theme.mod_poll_ms, NULL);
}

void ui_tile_keys_dress(lv_obj_t *obj, const ui_tile_t *tile)
{
    if (tile->action.type != TD_ACTION_MOD) {
        return;
    }
    const ui_styles_t *st = ui_styles();
    const ui_theme_t *t = &ui_theme;
    lv_subject_t *subject = s_mod[tile->action.mod.mod];

    lv_obj_add_style(obj, &st->mod_one_shot, LV_STATE_USER_1);
    lv_obj_add_style(obj, &st->mod_locked, LV_STATE_CHECKED);
    lv_subject_add_observer_obj(subject, apply_look, obj, NULL);

    add_glyph_part(obj, subject, &st->lock_shackle, t->lock.shackle_w, t->lock.shackle_h,
                   -(t->lock.inset + (t->lock.w - t->lock.shackle_w) / 2), (t->lock.shackle_h - t->lock.h) / 2);
    add_glyph_part(obj, subject, &st->lock_body, t->lock.w, t->lock.body_h, -t->lock.inset,
                   (t->lock.h - t->lock.body_h) / 2);
}

static key_sink_t s_hid;
void ui_set_hid_sink(key_sink_t sink) { s_hid = sink; }
static void click_hid(void *ctx, const td_action_t *action)
{
    (void)ctx;
    ui_action_hid(action, s_hid);
}

static void click_key(void *ctx, uint8_t usage, uint8_t mods)
{
    (void)ctx;
    if (mods != 0) {
        keys_chord(usage, mods);
    } else {
        keys_tap(usage);
    }
}

static void click_mod(void *ctx, key_mod_t mod)
{
    (void)ctx;
    keys_mod(mod);
}

static void click_consumer(void *ctx, uint16_t usage)
{
    (void)ctx;
    keys_consumer(usage);
}

static void click_page(void *ctx, uint8_t index)
{
    (void)ctx;
    ui_goto(index);
}

static ui_press_hook_t s_press_hook;

void ui_set_press_hook(ui_press_hook_t hook)
{
    s_press_hook = hook;
}

static void click_press(void *ctx, const ui_tile_t *tile)
{
    const ui_page_t *page = ctx;
    const char *page_id = page != NULL ? page->id : "?";
    const char *kind = td_action_name(&tile->action);
    if (s_press_hook != NULL) {
        s_press_hook(page_id, tile->id, kind);
    } else {
        ESP_LOGI("ui", "press %s/%s %s", page_id, tile->id, kind);
    }
}

static ui_part_hook_t s_part_hook;

void ui_set_part_hook(ui_part_hook_t hook)
{
    s_part_hook = hook;
}

void ui_tile_keys_press_part(const char *page_id, const ui_tile_t *tile, const char *part, uint32_t generation)
{
    const char *kind = td_action_name(&tile->action);
    if (s_part_hook != NULL) {
        s_part_hook(page_id, tile->id, kind, part, generation);
    } else {
        ESP_LOGI("ui", "press %s/%s %s %s", page_id, tile->id, kind, part);
    }
}

void ui_tile_keys_click(const ui_page_t *page, const ui_tile_t *tile)
{
    static const ui_action_ops_t ops = {
        .hid = click_hid, .key = click_key, .mod = click_mod, .consumer = click_consumer, .page = click_page, .press = click_press,
    };
    if (ui_tile_click(tile, &ops, (void *)page)) {
        sync_mods();
    }
}

void ui_tile_keys_page_changed(void)
{
    keys_clear_one_shot();
    sync_mods();
}
