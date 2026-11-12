#include <string.h>
#include "ui_pages.h"

static bool has_text(const char *s)
{
    return s != NULL && s[0] != '\0';
}

static bool grid_size_ok(uint8_t n)
{
    return n >= 1 && n <= UI_GRID_MAX;
}

static bool tile_fits(const ui_page_t *page, const ui_tile_t *tile)
{
    return grid_size_ok(page->rows) && grid_size_ok(page->cols) && tile->row_span >= 1 && tile->col_span >= 1 &&
           tile->row + tile->row_span <= page->rows && tile->col + tile->col_span <= page->cols;
}

static bool tiles_overlap(const ui_tile_t *a, const ui_tile_t *b)
{
    return a->row < b->row + b->row_span && b->row < a->row + a->row_span && a->col < b->col + b->col_span &&
           b->col < a->col + a->col_span;
}

bool ui_page_valid(const ui_page_t *page)
{
    if (page == NULL || !has_text(page->id) || !has_text(page->title) || !grid_size_ok(page->rows) ||
        !grid_size_ok(page->cols)) {
        return false;
    }
    if (page->tile_count > 0 && page->tiles == NULL) {
        return false;
    }
    for (size_t i = 0; i < page->tile_count; i++) {
        const ui_tile_t *tile = &page->tiles[i];
        if (!has_text(tile->id) || !has_text(tile->label) || !tile_fits(page, tile)) {
            return false;
        }
        for (size_t j = 0; j < i; j++) {
            if (tiles_overlap(tile, &page->tiles[j])) {
                return false;
            }
        }
    }
    return true;
}

/* The n-th cell edge from the area's start, counting each cell with the gap that follows it. */
static int32_t cell_edge(int32_t length, int32_t gap, int32_t cells, int32_t n)
{
    return n * (length + gap) / cells;
}

ui_rect_t ui_tile_rect(const ui_page_t *page, const ui_tile_t *tile, ui_rect_t area, int16_t gap)
{
    if (page == NULL || tile == NULL || !tile_fits(page, tile)) {
        return (ui_rect_t){ 0, 0, 0, 0 };
    }

    const int32_t x0 = cell_edge(area.w, gap, page->cols, tile->col);
    const int32_t x1 = cell_edge(area.w, gap, page->cols, tile->col + tile->col_span) - gap;
    const int32_t y0 = cell_edge(area.h, gap, page->rows, tile->row);
    const int32_t y1 = cell_edge(area.h, gap, page->rows, tile->row + tile->row_span) - gap;
    return (ui_rect_t){
        .x = (int16_t)(area.x + x0),
        .y = (int16_t)(area.y + y0),
        .w = (int16_t)(x1 - x0),
        .h = (int16_t)(y1 - y0),
    };
}

const ui_tile_t *ui_page_find_tile(const ui_page_t *page, const char *id)
{
    if (page == NULL || id == NULL) {
        return NULL;
    }
    for (size_t i = 0; i < page->tile_count; i++) {
        if (strcmp(page->tiles[i].id, id) == 0) {
            return &page->tiles[i];
        }
    }
    return NULL;
}

const char *td_action_name(const td_action_t *action)
{
    if (action == NULL) {
        return "none";
    }
    switch (action->type) {
    case TD_ACTION_KEY:
        return "key";
    case TD_ACTION_MOD:
        return "mod";
    case TD_ACTION_CONSUMER:
        return "consumer";
    case TD_ACTION_PAGE:
        return "page";
    case TD_ACTION_APP:
        return action->app.kind == TD_APP_LAUNCH ? "launch" : action->app.kind == TD_APP_OPEN ? "open" : "command";
    case TD_ACTION_WIDGET:
        return "widget";
    case TD_ACTION_NONE:
        break;
    }
    return "none";
}

bool ui_tile_click(const ui_tile_t *tile, const ui_action_ops_t *ops, void *ctx)
{
    if (tile == NULL || ops == NULL) {
        return false;
    }
    const td_action_t *a = &tile->action;
    if (a->hid && (a->type == TD_ACTION_KEY || a->type == TD_ACTION_CONSUMER)) {
        if (!ops->hid) return false;
        ops->hid(ctx, a);
        return true;
    }
    switch (a->type) {
    case TD_ACTION_KEY:
        if (ops->key == NULL) {
            return false;
        }
        ops->key(ctx, a->key.usage, a->key.mods);
        return true;
    case TD_ACTION_MOD:
        if (ops->mod == NULL) {
            return false;
        }
        ops->mod(ctx, a->mod.mod);
        return true;
    case TD_ACTION_CONSUMER:
        if (ops->consumer == NULL) {
            return false;
        }
        ops->consumer(ctx, a->consumer.usage);
        return true;
    case TD_ACTION_PAGE:
        if (ops->page == NULL) {
            return false;
        }
        ops->page(ctx, a->page.index);
        return true;
    case TD_ACTION_APP:
    case TD_ACTION_WIDGET:
        if (ops->press == NULL) {
            return false;
        }
        ops->press(ctx, tile);
        return true;
    case TD_ACTION_NONE:
        break;
    }
    return false;
}

void ui_action_hid(const td_action_t *action, key_sink_t sink)
{
    if (!action || !sink.report) return;
    key_report_t down, up;
    if (action->type == TD_ACTION_KEY) {
        down = key_report_kbd(action->key.mods, action->key.usage);
        up = key_report_kbd(0, 0);
    } else if (action->type == TD_ACTION_CONSUMER) {
        down = key_report_con(action->consumer.usage);
        up = key_report_con(0);
    } else return;
    sink.report(sink.ctx, &down);
    sink.report(sink.ctx, &up);
}
