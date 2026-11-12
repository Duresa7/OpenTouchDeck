#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "cJSON.h"
#include "td_layout.h"
#include "ui_live.h"

#ifdef ESP_PLATFORM
#include "esp_heap_caps.h"
#include "sdkconfig.h"
#endif

/*
 * The parser follows schema/check-layouts.mjs. The file's text is checked first, in this order: UTF-8, JSON grammar
 * and nesting, then the rules JSON.parse lets through, which are integer form, lone surrogates and U+0000. cJSON then
 * builds the tree, which is checked for repeated members, then against the schema, then for the rules the schema
 * cannot express. The first broken rule is reported, so a file that breaks one rule reports that rule's key.
 */

static void *arena_alloc(size_t size)
{
#ifdef ESP_PLATFORM
    return heap_caps_malloc_prefer(size, 2, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT, MALLOC_CAP_8BIT);
#else
    return malloc(size);
#endif
}

static bool fail(td_layout_error_t *err, const char *key, const char *fmt, ...)
{
    snprintf(err->key, sizeof err->key, "%s", key);
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(err->message, sizeof err->message, fmt, ap);
    va_end(ap);
    return false;
}

/* A message quotes member names from the file, so it is cut to printable ASCII before a reply carries it. */
static void sanitize(td_layout_error_t *err)
{
    for (char *c = err->message; *c != '\0'; c++) {
        if ((unsigned char)*c < 0x20 || (unsigned char)*c > 0x7e) {
            *c = '?';
        }
    }
}

/* UTF-8 as a fatal TextDecoder reads it: no overlong forms, no surrogates, nothing above U+10FFFF. */
static bool utf8_valid(const uint8_t *p, size_t n)
{
    size_t i = 0;
    while (i < n) {
        const uint8_t c = p[i];
        if (c < 0x80) {
            i++;
            continue;
        }
        size_t need = 0;
        uint8_t lo = 0x80;
        uint8_t hi = 0xbf;
        if (c >= 0xc2 && c <= 0xdf) {
            need = 1;
        } else if (c >= 0xe0 && c <= 0xef) {
            need = 2;
            lo = c == 0xe0 ? 0xa0 : 0x80;
            hi = c == 0xed ? 0x9f : 0xbf;
        } else if (c >= 0xf0 && c <= 0xf4) {
            need = 3;
            lo = c == 0xf0 ? 0x90 : 0x80;
            hi = c == 0xf4 ? 0x8f : 0xbf;
        } else {
            return false;
        }
        if (n - i - 1 < need || p[i + 1] < lo || p[i + 1] > hi) {
            return false;
        }
        for (size_t k = 2; k <= need; k++) {
            if (p[i + k] < 0x80 || p[i + k] > 0xbf) {
                return false;
            }
        }
        i += need + 1;
    }
    return true;
}

/* The grammar of JSON.parse, which is stricter than cJSON's: only four whitespace bytes, no trailing text, no raw
 * control bytes in strings, and exact number and literal forms. A rule that JSON.parse lets through goes in `soft`,
 * and only the first one is kept. */
typedef struct {
    const uint8_t *p;
    const uint8_t *start;
    const uint8_t *end;
    int depth;
    bool too_deep;
    bool has_soft;
    td_layout_error_t soft;
} scan_t;

static void soft(scan_t *s, const char *key, const char *message)
{
    if (!s->has_soft) {
        s->has_soft = true;
        fail(&s->soft, key, "%s near byte %u", message, (unsigned)(s->p - s->start));
    }
}

static void skip_ws(scan_t *s)
{
    while (s->p < s->end && (*s->p == ' ' || *s->p == '\t' || *s->p == '\n' || *s->p == '\r')) {
        s->p++;
    }
}

static int hex4(const uint8_t *p)
{
    int v = 0;
    for (int i = 0; i < 4; i++) {
        const uint8_t c = p[i];
        int d = -1;
        if (c >= '0' && c <= '9') {
            d = c - '0';
        } else if (c >= 'a' && c <= 'f') {
            d = c - 'a' + 10;
        } else if (c >= 'A' && c <= 'F') {
            d = c - 'A' + 10;
        }
        if (d < 0) {
            return -1;
        }
        v = v * 16 + d;
    }
    return v;
}

static bool scan_string(scan_t *s)
{
    s->p++;
    while (s->p < s->end) {
        uint8_t c = *s->p++;
        if (c == '"') {
            return true;
        }
        if (c < 0x20) {
            return false;
        }
        if (c != '\\') {
            continue;
        }
        if (s->p >= s->end) {
            return false;
        }
        c = *s->p++;
        if (c == '"' || c == '\\' || c == '/' || c == 'b' || c == 'f' || c == 'n' || c == 'r' || c == 't') {
            continue;
        }
        if (c != 'u' || s->end - s->p < 4) {
            return false;
        }
        const int cp = hex4(s->p);
        if (cp < 0) {
            return false;
        }
        s->p += 4;
        if (cp == 0) {
            soft(s, "string.control-character", "a string holds U+0000");
        } else if (cp >= 0xd800 && cp <= 0xdbff) {
            if (s->end - s->p >= 6 && s->p[0] == '\\' && s->p[1] == 'u') {
                const int low = hex4(s->p + 2);
                if (low >= 0xdc00 && low <= 0xdfff) {
                    s->p += 6;
                    continue;
                }
            }
            soft(s, "string.malformed-unicode", "a string holds a lone surrogate");
        } else if (cp >= 0xdc00 && cp <= 0xdfff) {
            soft(s, "string.malformed-unicode", "a string holds a lone surrogate");
        }
    }
    return false;
}

static bool is_digit(const scan_t *s)
{
    return s->p < s->end && *s->p >= '0' && *s->p <= '9';
}

static bool scan_digits(scan_t *s)
{
    if (!is_digit(s)) {
        return false;
    }
    while (is_digit(s)) {
        s->p++;
    }
    return true;
}

/* An integer is 0, or an optional minus and a nonzero digit and more digits. A fraction, an exponent or -0 is valid
 * JSON but breaks json.integer-form. */
static bool scan_number(scan_t *s)
{
    const uint8_t *start = s->p;
    bool integer = true;
    if (*s->p == '-') {
        s->p++;
    }
    if (s->p < s->end && *s->p == '0') {
        s->p++;
    } else if (!scan_digits(s)) {
        return false;
    }
    if (s->p < s->end && *s->p == '.') {
        integer = false;
        s->p++;
        if (!scan_digits(s)) {
            return false;
        }
    }
    if (s->p < s->end && (*s->p == 'e' || *s->p == 'E')) {
        integer = false;
        s->p++;
        if (s->p < s->end && (*s->p == '+' || *s->p == '-')) {
            s->p++;
        }
        if (!scan_digits(s)) {
            return false;
        }
    }
    if (!integer || (s->p - start == 2 && start[0] == '-' && start[1] == '0')) {
        const uint8_t *after = s->p;
        s->p = start;
        soft(s, "json.integer-form", "a number is not written as a JSON integer");
        s->p = after;
    }
    return true;
}

static bool scan_literal(scan_t *s, const char *word)
{
    const size_t n = strlen(word);
    if ((size_t)(s->end - s->p) < n || memcmp(s->p, word, n) != 0) {
        return false;
    }
    s->p += n;
    return true;
}

static bool scan_value(scan_t *s);

static bool scan_container(scan_t *s, uint8_t close)
{
    if (++s->depth > TD_LAYOUT_MAX_DEPTH) {
        s->too_deep = true;
        return false;
    }
    s->p++;
    skip_ws(s);
    if (s->p < s->end && *s->p == close) {
        s->p++;
        s->depth--;
        return true;
    }
    for (;;) {
        if (close == '}') {
            skip_ws(s);
            if (s->p >= s->end || *s->p != '"' || !scan_string(s)) {
                return false;
            }
            skip_ws(s);
            if (s->p >= s->end || *s->p != ':') {
                return false;
            }
            s->p++;
        }
        if (!scan_value(s)) {
            return false;
        }
        skip_ws(s);
        if (s->p >= s->end) {
            return false;
        }
        if (*s->p == ',') {
            s->p++;
            continue;
        }
        if (*s->p != close) {
            return false;
        }
        s->p++;
        s->depth--;
        return true;
    }
}

static bool scan_value(scan_t *s)
{
    skip_ws(s);
    if (s->p >= s->end) {
        return false;
    }
    switch (*s->p) {
    case '{':
        return scan_container(s, '}');
    case '[':
        return scan_container(s, ']');
    case '"':
        return scan_string(s);
    case 't':
        return scan_literal(s, "true");
    case 'f':
        return scan_literal(s, "false");
    case 'n':
        return scan_literal(s, "null");
    default:
        if (*s->p == '-' || (*s->p >= '0' && *s->p <= '9')) {
            return scan_number(s);
        }
        return false;
    }
}

static bool scan_text(const char *json, size_t len, td_layout_error_t *err)
{
    scan_t s = { .p = (const uint8_t *)json, .start = (const uint8_t *)json, .end = (const uint8_t *)json + len };
    bool ok = scan_value(&s);
    if (ok) {
        skip_ws(&s);
        ok = s.p == s.end;
    }
    if (!ok) {
        if (s.too_deep) {
            return fail(err, "file.json", "JSON nests deeper than %d levels", TD_LAYOUT_MAX_DEPTH);
        }
        return fail(err, "file.json", "the file is not valid JSON near byte %u", (unsigned)(s.p - s.start));
    }
    if (s.has_soft) {
        *err = s.soft;
        return false;
    }
    return true;
}

static bool check_duplicates(const cJSON *item, td_layout_error_t *err)
{
    for (const cJSON *c = item->child; c != NULL; c = c->next) {
        if (cJSON_IsObject(item)) {
            for (const cJSON *d = item->child; d != c; d = d->next) {
                if (strcmp(d->string, c->string) == 0) {
                    return fail(err, "json.duplicate-member", "an object repeats the member name \"%s\"", c->string);
                }
            }
        }
        if (!check_duplicates(c, err)) {
            return false;
        }
    }
    return true;
}

static const cJSON *member(const cJSON *obj, const char *name)
{
    return cJSON_GetObjectItemCaseSensitive(obj, name);
}

/* The first `required` names must be present, and the rest may be. */
static bool check_some_members(const cJSON *obj, const char *where, const char *const *names, size_t count,
                               size_t required, td_layout_error_t *err)
{
    for (const cJSON *c = obj->child; c != NULL; c = c->next) {
        bool known = false;
        for (size_t i = 0; i < count && !known; i++) {
            known = strcmp(c->string, names[i]) == 0;
        }
        if (!known) {
            return fail(err, "shape.unknown-field", "%s must NOT have additional properties \"%s\"", where, c->string);
        }
    }
    for (size_t i = 0; i < required; i++) {
        if (member(obj, names[i]) == NULL) {
            return fail(err, "shape.missing-field", "%s must have required property \"%s\"", where, names[i]);
        }
    }
    return true;
}

static bool check_members(const cJSON *obj, const char *where, const char *const *names, size_t count,
                          td_layout_error_t *err)
{
    return check_some_members(obj, where, names, count, count, err);
}

/* Unicode White_Space, which the schema's \p{White_Space} matches. */
static bool is_white(uint32_t cp)
{
    return (cp >= 0x09 && cp <= 0x0d) || cp == 0x20 || cp == 0x85 || cp == 0xa0 || cp == 0x1680 ||
           (cp >= 0x2000 && cp <= 0x200a) || cp == 0x2028 || cp == 0x2029 || cp == 0x202f || cp == 0x205f ||
           cp == 0x3000;
}

/* Decodes one code point of UTF-8 that utf8_valid already accepted. */
static uint32_t next_cp(const unsigned char **p)
{
    const unsigned char *s = *p;
    uint32_t cp;
    size_t n;
    if (s[0] < 0x80) {
        cp = s[0];
        n = 1;
    } else if (s[0] < 0xe0) {
        cp = s[0] & 0x1f;
        n = 2;
    } else if (s[0] < 0xf0) {
        cp = s[0] & 0x0f;
        n = 3;
    } else {
        cp = s[0] & 0x07;
        n = 4;
    }
    for (size_t i = 1; i < n; i++) {
        cp = (cp << 6) | (s[i] & 0x3f);
    }
    *p = s + n;
    return cp;
}

/* A text field: a string with no U+0001 to U+001F or U+007F, at most max_cp code points when max_cp is not 0, with a
 * character that is not whitespace when blank_key is set, and not empty when empty_key is set. U+0000 never gets here,
 * because the scanner refused it. */
static bool check_text(const cJSON *v, const char *where, size_t max_cp, const char *long_key, const char *blank_key,
                       const char *empty_key, td_layout_error_t *err)
{
    if (!cJSON_IsString(v)) {
        return fail(err, "shape.type", "%s must be string", where);
    }
    size_t count = 0;
    bool blank = true;
    for (const unsigned char *p = (const unsigned char *)v->valuestring; *p != '\0'; count++) {
        if (*p < 0x20 || *p == 0x7f) {
            return fail(err, "string.control-character", "%s holds a control character", where);
        }
        const bool white = is_white(next_cp(&p));
        blank = blank && white;
    }
    if (max_cp > 0 && count > max_cp) {
        return fail(err, long_key, "%s must NOT have more than %u characters", where, (unsigned)max_cp);
    }
    if (blank_key != NULL && blank) {
        return fail(err, blank_key, "%s must hold a character that is not whitespace", where);
    }
    if (empty_key != NULL && count == 0) {
        return fail(err, empty_key, "%s must NOT have fewer than 1 characters", where);
    }
    return true;
}

static bool check_id(const cJSON *v, const char *where, td_layout_error_t *err)
{
    if (!cJSON_IsString(v)) {
        return fail(err, "shape.type", "%s must be string", where);
    }
    const char *s = v->valuestring;
    size_t n = 0;
    for (; s[n] != '\0'; n++) {
        const char c = s[n];
        if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_')) {
            n = 0;
            break;
        }
    }
    if (n < 1 || n > 15) {
        return fail(err, "id.format", "%s must match pattern \"^[a-z0-9_]{1,15}$\"", where);
    }
    return true;
}

static bool is_integer(const cJSON *v)
{
    return cJSON_IsNumber(v) && isfinite(v->valuedouble) && floor(v->valuedouble) == v->valuedouble;
}

static bool check_int(const cJSON *v, const char *where, double min, double max, const char *range_key,
                      td_layout_error_t *err)
{
    if (!is_integer(v)) {
        return fail(err, "shape.type", "%s must be integer", where);
    }
    if (v->valuedouble < min || v->valuedouble > max) {
        return fail(err, range_key, "%s must be from %.0f to %.0f", where, min, max);
    }
    return true;
}

static int modifier_index(const cJSON *v)
{
    static const char *const names[KEY_MOD_COUNT] = { "ctrl", "shift", "alt", "win" };
    if (!cJSON_IsString(v)) {
        return -1;
    }
    for (int i = 0; i < KEY_MOD_COUNT; i++) {
        if (strcmp(v->valuestring, names[i]) == 0) {
            return i;
        }
    }
    return -1;
}

static bool icon_valid(const char *s)
{
    static const char *const lv[] = { "lv:prev", "lv:play_pause", "lv:next", "lv:mute", "lv:volume_mid", "lv:volume_max" };
    for (size_t i = 0; i < sizeof lv / sizeof lv[0]; i++) {
        if (strcmp(s, lv[i]) == 0) {
            return true;
        }
    }
    if (strncmp(s, "file:", 5) != 0 || strlen(s) != 21) {
        return false;
    }
    for (const char *c = s + 5; *c != '\0'; c++) {
        if (!((*c >= '0' && *c <= '9') || (*c >= 'a' && *c <= 'f'))) {
            return false;
        }
    }
    return true;
}

static bool check_args(const cJSON *v, const char *where, td_layout_error_t *err)
{
    if (!cJSON_IsArray(v)) {
        return fail(err, "shape.type", "%s must be array", where);
    }
    for (const cJSON *a = v->child; a != NULL; a = a->next) {
        if (!check_text(a, where, 0, NULL, NULL, NULL, err)) {
            return false;
        }
    }
    return true;
}

static bool check_mods(const cJSON *v, const char *where, td_layout_error_t *err)
{
    if (!cJSON_IsArray(v)) {
        return fail(err, "shape.type", "%s must be array", where);
    }
    unsigned seen = 0;
    for (const cJSON *m = v->child; m != NULL; m = m->next) {
        const int mod = modifier_index(m);
        if (mod < 0) {
            return fail(err, "modifier.unknown", "%s must be ctrl, shift, alt or win", where);
        }
        if (seen & (1u << mod)) {
            return fail(err, "key.mods-duplicate", "%s must NOT have duplicate items", where);
        }
        seen |= 1u << mod;
    }
    return true;
}

typedef struct {
    const char *type;
    const char *members[5];
    size_t count;
} action_shape_t;

static const action_shape_t k_actions[] = {
    { "key", { "type", "usage", "mods" }, 3 },
    { "mod", { "type", "mod" }, 2 },
    { "consumer", { "type", "usage" }, 2 },
    { "page", { "type", "page" }, 2 },
    { "none", { "type" }, 1 },
    { "launch", { "type", "path", "args", "cwd" }, 4 },
    { "open", { "type", "target" }, 2 },
    { "command", { "type", "program", "args", "cwd", "shell" }, 5 },
    { "widget", { "type", "source", "field", "account" }, 4 },
};

static const action_shape_t *action_shape(const cJSON *action)
{
    const cJSON *type = member(action, "type");
    if (!cJSON_IsString(type)) {
        return NULL;
    }
    for (size_t i = 0; i < sizeof k_actions / sizeof k_actions[0]; i++) {
        if (strcmp(type->valuestring, k_actions[i].type) == 0) {
            return &k_actions[i];
        }
    }
    return NULL;
}

static bool check_action(const cJSON *a, const char *tile_where, td_layout_error_t *err)
{
    char where[64];
    char field[96];
    snprintf(where, sizeof where, "%s/action", tile_where);
    if (!cJSON_IsObject(a)) {
        return fail(err, "shape.type", "%s must be object", where);
    }
    const action_shape_t *shape = action_shape(a);
    if (shape == NULL) {
        return fail(err, "action.type", "%s/type must name one of the nine action types", where);
    }
    const bool key_action = strcmp(shape->type, "key") == 0 || strcmp(shape->type, "consumer") == 0;
    const char *names[6];
    memcpy(names, shape->members, shape->count * sizeof names[0]);
    names[shape->count] = "sink";
    if (!check_some_members(a, where, names, shape->count + (key_action ? 1 : 0), shape->count, err)) {
        return false;
    }
#define FIELD(name) (snprintf(field, sizeof field, "%s/%s", where, name), field)
    const cJSON *sink = member(a, "sink");
    if (sink && (!cJSON_IsString(sink) || (strcmp(sink->valuestring, "link") != 0 && strcmp(sink->valuestring, "hid") != 0))) {
        return fail(err, "action.sink", "%s must be link or hid", FIELD("sink"));
    }
    const bool hid = sink && strcmp(sink->valuestring, "hid") == 0;
#if !CONFIG_TD_RADIO_BUILD
    if (hid) return fail(err, "action.sink", "%s is unavailable on this Device", FIELD("sink"));
#endif
    if (hid && strcmp(shape->type, "consumer") == 0 && member(a, "usage") && member(a, "usage")->valuedouble > 1023) {
        return fail(err, "consumer.usage-range", "%s exceeds Bluetooth's consumer range", FIELD("usage"));
    }
    const char *t = shape->type;
    if (strcmp(t, "key") == 0) {
        return check_int(member(a, "usage"), FIELD("usage"), 1, 255, "key.usage-range", err) &&
               check_mods(member(a, "mods"), FIELD("mods"), err);
    }
    if (strcmp(t, "mod") == 0) {
        return modifier_index(member(a, "mod")) >= 0 ||
               fail(err, "modifier.unknown", "%s must be ctrl, shift, alt or win", FIELD("mod"));
    }
    if (strcmp(t, "consumer") == 0) {
        return check_int(member(a, "usage"), FIELD("usage"), 1, 65535, "consumer.usage-range", err);
    }
    if (strcmp(t, "page") == 0) {
        return check_id(member(a, "page"), FIELD("page"), err);
    }
    if (strcmp(t, "launch") == 0) {
        return check_text(member(a, "path"), FIELD("path"), 0, NULL, NULL, "text.empty", err) &&
               check_args(member(a, "args"), FIELD("args"), err) &&
               check_text(member(a, "cwd"), FIELD("cwd"), 0, NULL, NULL, NULL, err);
    }
    if (strcmp(t, "open") == 0) {
        return check_text(member(a, "target"), FIELD("target"), 0, NULL, NULL, "text.empty", err);
    }
    if (strcmp(t, "command") == 0) {
        if (!cJSON_IsBool(member(a, "shell"))) {
            return fail(err, "shape.type", "%s must be boolean", FIELD("shell"));
        }
        return check_text(member(a, "program"), FIELD("program"), 0, NULL, NULL, "text.empty", err) &&
               check_args(member(a, "args"), FIELD("args"), err) &&
               check_text(member(a, "cwd"), FIELD("cwd"), 0, NULL, NULL, NULL, err);
    }
    if (strcmp(t, "widget") == 0) {
        return check_text(member(a, "source"), FIELD("source"), 0, NULL, NULL, "text.empty", err) &&
               check_text(member(a, "field"), FIELD("field"), 0, NULL, NULL, "text.empty", err) &&
               check_text(member(a, "account"), FIELD("account"), 0, NULL, NULL, NULL, err);
    }
#undef FIELD
    return true;
}

static const char *const k_stream_types[] = { "number", "series", "text", "image" };

static int stream_type(const cJSON *v)
{
    for (int i = 0; cJSON_IsString(v) && i < (int)(sizeof k_stream_types / sizeof k_stream_types[0]); i++) {
        if (strcmp(v->valuestring, k_stream_types[i]) == 0) {
            return i;
        }
    }
    return -1;
}

typedef struct {
    const char *render;
    const char *id;
    unsigned types;
    bool required;
} stream_rule_t;

static const stream_rule_t k_streams[] = {
    { "graph", "value", (1u << UI_STREAM_NUMBER) | (1u << UI_STREAM_SERIES), true },
    { "bar", "value", 1u << UI_STREAM_NUMBER, true },
    { "bar", "muted", 1u << UI_STREAM_NUMBER, false },
    { "now-playing", "title", 1u << UI_STREAM_TEXT, true },
    { "now-playing", "artist", 1u << UI_STREAM_TEXT, true },
    { "now-playing", "cover", 1u << UI_STREAM_IMAGE, true },
    { "now-playing", "state", 1u << UI_STREAM_NUMBER, true },
};

static bool check_streams(const cJSON *tile, td_layout_error_t *err)
{
    const char *render = cJSON_GetStringValue(member(tile, "render"));
    const cJSON *streams = member(tile, "streams");
    for (const cJSON *s = streams ? streams->child : NULL; s; s = s->next) {
        bool valid = false;
        for (size_t i = 0; render && i < sizeof k_streams / sizeof k_streams[0]; i++) {
            const stream_rule_t *rule = &k_streams[i];
            if (strcmp(render, rule->render) == 0 && strcmp(s->string, rule->id) == 0) {
                valid = (rule->types & (1u << stream_type(s))) != 0;
            }
        }
        if (!valid) return fail(err, "tile.streams", "renderer cannot bind stream %s", s->string);
    }
    for (size_t i = 0; render && i < sizeof k_streams / sizeof k_streams[0]; i++) {
        const stream_rule_t *rule = &k_streams[i];
        if (strcmp(render, rule->render) == 0 && rule->required && !member(streams, rule->id)) {
            return fail(err, "tile.streams", "renderer needs stream %s", rule->id);
        }
    }
    return true;
}

static bool check_tile(const cJSON *t, int page, int index, bool v2, td_layout_error_t *err)
{
    static const char *const members[] = { "id", "label", "row", "col", "rowSpan", "colSpan", "icon", "action", "asset", "render", "streams", "input" };
    char where[48];
    char field[64];
    snprintf(where, sizeof where, "/pages/%d/tiles/%d", page, index);
    if (!cJSON_IsObject(t)) {
        return fail(err, "shape.type", "%s must be object", where);
    }
    if (!check_some_members(t, where, members, v2 ? 12 : 8, 8, err)) {
        return false;
    }
#define FIELD(name) (snprintf(field, sizeof field, "%s/%s", where, name), field)
    if (!check_id(member(t, "id"), FIELD("id"), err) ||
        !check_text(member(t, "label"), FIELD("label"), 16, "label.too-long", "label.blank", NULL, err) ||
        !check_int(member(t, "row"), FIELD("row"), 0, 7, "tile.position", err) ||
        !check_int(member(t, "col"), FIELD("col"), 0, 7, "tile.position", err) ||
        !check_int(member(t, "rowSpan"), FIELD("rowSpan"), 1, 8, "tile.span", err) ||
        !check_int(member(t, "colSpan"), FIELD("colSpan"), 1, 8, "tile.span", err)) {
        return false;
    }
    const cJSON *icon = member(t, "icon");
    if (!cJSON_IsNull(icon)) {
        if (!cJSON_IsString(icon)) {
            return fail(err, "shape.type", "%s must be string,null", FIELD("icon"));
        }
        if (!icon_valid(icon->valuestring)) {
            return fail(err, "icon.format", "%s must be null, an lv: name or file: and 16 lowercase hex digits",
                        FIELD("icon"));
        }
    }
#undef FIELD
    const cJSON *asset = member(t, "asset");
    if (asset != NULL && !cJSON_IsNull(asset)) {
        if (!cJSON_IsNull(icon) || !cJSON_IsString(asset) || strlen(asset->valuestring) != 71 || strncmp(asset->valuestring, "sha256:", 7) != 0) {
            return fail(err, "icon.format", "%s/asset must be a SHA-256 reference with icon null", where);
        }
        for (const char *c = asset->valuestring + 7; *c; c++) {
            if (!((*c >= '0' && *c <= '9') || (*c >= 'a' && *c <= 'f'))) {
                return fail(err, "icon.format", "%s/asset must use lowercase hex", where);
            }
        }
    }
    const cJSON *render = member(t, "render");
    const bool gif = cJSON_IsString(render) && strcmp(render->valuestring, "gif") == 0;
    const bool graph = cJSON_IsString(render) && strcmp(render->valuestring, "graph") == 0;
    const bool bar = cJSON_IsString(render) && strcmp(render->valuestring, "bar") == 0;
    const bool media = cJSON_IsString(render) && strcmp(render->valuestring, "now-playing") == 0;
    if (render && !graph && !bar && !media && (!gif || !cJSON_IsString(asset))) {
        return fail(err, "icon.format", "%s/render must be graph, or gif with an Asset reference", where);
    }
    if ((bar || media) && asset && !cJSON_IsNull(asset)) {
        return fail(err, "icon.format", "%s/render cannot use a static Asset", where);
    }
    const cJSON *input = member(t, "input");
    const char *action_type = cJSON_GetStringValue(member(member(t, "action"), "type"));
    if (input && (!cJSON_IsTrue(input) || !bar || !action_type || strcmp(action_type, "widget") != 0)) {
        return fail(err, "tile.input", "%s/input requires true on a bar with a widget action", where);
    }
    const cJSON *streams = member(t, "streams");
    if (streams != NULL) {
        if (!cJSON_IsObject(streams)) {
            return fail(err, "shape.type", "%s/streams must be object", where);
        }
        for (const cJSON *s = streams->child; s != NULL; s = s->next) {
            if (stream_type(s) < 0) {
                return fail(err, "tile.streams", "%s/streams types must be number, series, text or image", where);
            }
        }
    }
    return check_action(member(t, "action"), where, err);
}

static bool is_kind(const cJSON *page, const char *kind)
{
    const cJSON *v = member(page, "kind");
    return cJSON_IsString(v) && strcmp(v->valuestring, kind) == 0;
}

static bool check_page(const cJSON *p, int index, bool v2, td_layout_error_t *err)
{
    static const char *const grid[] = { "id", "title", "kind", "rows", "cols", "tiles" };
    static const char *const dashboard[] = { "id", "title", "kind", "source" };
    char where[24];
    char field[48];
    snprintf(where, sizeof where, "/pages/%d", index);
    if (!cJSON_IsObject(p)) {
        return fail(err, "shape.type", "%s must be object", where);
    }
    const bool is_grid = is_kind(p, "grid");
    if (!is_grid && !is_kind(p, "dashboard")) {
        return fail(err, "page.kind", "%s/kind must be grid or dashboard", where);
    }
    if (!check_members(p, where, is_grid ? grid : dashboard, is_grid ? 6 : 4, err)) {
        return false;
    }
#define FIELD(name) (snprintf(field, sizeof field, "%s/%s", where, name), field)
    if (!check_id(member(p, "id"), FIELD("id"), err) ||
        !check_text(member(p, "title"), FIELD("title"), 24, "title.too-long", "title.blank", NULL, err)) {
        return false;
    }
    if (!is_grid) {
        return check_text(member(p, "source"), FIELD("source"), 0, NULL, NULL, "text.empty", err);
    }
    if (!check_int(member(p, "rows"), FIELD("rows"), 1, 8, "grid.size", err) ||
        !check_int(member(p, "cols"), FIELD("cols"), 1, 8, "grid.size", err)) {
        return false;
    }
    const cJSON *tiles = member(p, "tiles");
    if (!cJSON_IsArray(tiles)) {
        return fail(err, "shape.type", "%s must be array", FIELD("tiles"));
    }
    if (cJSON_GetArraySize(tiles) > TD_LAYOUT_MAX_TILES) {
        return fail(err, "page.tile-count", "%s must NOT have more than %d items", FIELD("tiles"), TD_LAYOUT_MAX_TILES);
    }
#undef FIELD
    int i = 0;
    for (const cJSON *t = tiles->child; t != NULL; t = t->next, i++) {
        if (!check_tile(t, index, i, v2, err)) {
            return false;
        }
    }
    return true;
}

static bool check_schema(const cJSON *root, td_layout_error_t *err)
{
    static const char *const members[] = { "schema", "name", "orientation", "pages", "fullScreen" };
    if (!cJSON_IsObject(root)) {
        return fail(err, "shape.type", "/ must be object");
    }
    if (!check_some_members(root, "/", members, sizeof members / sizeof members[0], 4, err)) {
        return false;
    }
    const cJSON *schema = member(root, "schema");
    if (!cJSON_IsNumber(schema) || (schema->valuedouble != 1 && schema->valuedouble != 2)) {
        return fail(err, "layout.schema-version", "/schema must be equal to constant 1");
    }
    if (!check_text(member(root, "name"), "/name", 24, "name.too-long", "name.blank", NULL, err)) {
        return false;
    }
    const cJSON *orientation = member(root, "orientation");
    if (!cJSON_IsString(orientation) ||
        (strcmp(orientation->valuestring, "landscape") != 0 && strcmp(orientation->valuestring, "portrait") != 0)) {
        return fail(err, "layout.orientation", "/orientation must be landscape or portrait");
    }
    const cJSON *full_screen = member(root, "fullScreen");
    if (full_screen != NULL && !cJSON_IsBool(full_screen)) {
        return fail(err, "shape.type", "/fullScreen must be boolean");
    }
    const cJSON *pages = member(root, "pages");
    if (!cJSON_IsArray(pages)) {
        return fail(err, "shape.type", "/pages must be array");
    }
    const int count = cJSON_GetArraySize(pages);
    if (count < 1 || count > TD_LAYOUT_MAX_PAGES) {
        return fail(err, "layout.page-count", "/pages must hold 1 to %d pages", TD_LAYOUT_MAX_PAGES);
    }
    int i = 0;
    for (const cJSON *p = pages->child; p != NULL; p = p->next, i++) {
        if (!check_page(p, i, schema->valuedouble == 2, err)) {
            return false;
        }
    }
    return true;
}

/* The UTF-8 length of JSON.stringify(value). Only `"` and `\` need escapes in a layout that passed the schema, but
 * every control character is counted as JSON.stringify writes it. */
static size_t string_size(const char *s)
{
    size_t n = 2;
    for (const unsigned char *p = (const unsigned char *)s; *p != '\0'; p++) {
        if (*p == '"' || *p == '\\' || *p == '\b' || *p == '\f' || *p == '\n' || *p == '\r' || *p == '\t') {
            n += 2;
        } else if (*p < 0x20) {
            n += 6;
        } else {
            n += 1;
        }
    }
    return n;
}

static size_t compact_size(const cJSON *v)
{
    if (cJSON_IsObject(v) || cJSON_IsArray(v)) {
        size_t n = 2;
        for (const cJSON *c = v->child; c != NULL; c = c->next) {
            if (c != v->child) {
                n++;
            }
            if (cJSON_IsObject(v)) {
                n += string_size(c->string) + 1;
            }
            n += compact_size(c);
        }
        return n;
    }
    if (cJSON_IsString(v)) {
        return string_size(v->valuestring);
    }
    if (cJSON_IsNumber(v)) {
        char buf[32];
        return (size_t)snprintf(buf, sizeof buf, "%.0f", v->valuedouble);
    }
    if (cJSON_IsTrue(v) || cJSON_IsNull(v)) {
        return 4;
    }
    return 5;
}

static int page_index(const cJSON *pages, const char *id)
{
    int i = 0;
    for (const cJSON *p = pages->child; p != NULL; p = p->next, i++) {
        if (strcmp(member(p, "id")->valuestring, id) == 0) {
            return i;
        }
    }
    return -1;
}

typedef struct {
    const char *id;
    int page;
    const char *page_id;
    int tile;
} seen_id_t;

static void describe(char *buf, size_t cap, const seen_id_t *at)
{
    if (at->tile < 0) {
        snprintf(buf, cap, "page %d \"%s\"", at->page, at->page_id);
    } else {
        snprintf(buf, cap, "page %d \"%s\" tile %d \"%s\"", at->page, at->page_id, at->tile, at->id);
    }
}

static bool add_id(seen_id_t *seen, size_t *count, seen_id_t at, td_layout_error_t *err)
{
    for (size_t i = 0; i < *count; i++) {
        if (strcmp(seen[i].id, at.id) == 0) {
            char here[48];
            char first[48];
            describe(here, sizeof here, &at);
            describe(first, sizeof first, &seen[i]);
            return fail(err, "id.duplicate", "duplicate id \"%s\" at %s; first used at %s", at.id, here, first);
        }
    }
    seen[(*count)++] = at;
    return true;
}

/* The rules that schema/check-layouts.mjs checks after the schema, in its order. */
static bool check_semantics(const cJSON *root, td_layout_error_t *err)
{
    const size_t compact = compact_size(root);
    if (compact > TD_LAYOUT_MAX_BYTES) {
        return fail(err, "layout.too-large", "layout compact UTF-8 JSON is %u bytes; maximum is %d bytes",
                    (unsigned)compact, TD_LAYOUT_MAX_BYTES);
    }
    const cJSON *pages = member(root, "pages");
    seen_id_t *seen = malloc(sizeof *seen * TD_LAYOUT_MAX_PAGES * (TD_LAYOUT_MAX_TILES + 1));
    if (seen == NULL) {
        return fail(err, "internal.no-memory", "no memory for the id check");
    }
    size_t count = 0;
    size_t live_images = 0;
    bool ok = true;
    int pi = 0;
    for (const cJSON *p = pages->child; ok && p != NULL; p = p->next, pi++) {
        const char *page_id = member(p, "id")->valuestring;
        ok = add_id(seen, &count, (seen_id_t){ page_id, pi, page_id, -1 }, err);
        if (!ok || is_kind(p, "dashboard")) {
            continue;
        }
        const int rows = member(p, "rows")->valueint;
        const int cols = member(p, "cols")->valueint;
        int owner[8][8];
        memset(owner, 0xff, sizeof owner);
        int ti = 0;
        for (const cJSON *t = member(p, "tiles")->child; ok && t != NULL; t = t->next, ti++) {
            const char *id = member(t, "id")->valuestring;
            ok = add_id(seen, &count, (seen_id_t){ id, pi, page_id, ti }, err);
            if (!ok) {
                break;
            }
            const cJSON *action = member(t, "action");
            if (strcmp(member(action, "type")->valuestring, "page") == 0) {
                const char *target = member(action, "page")->valuestring;
                if (page_index(pages, target) < 0) {
                    ok = fail(err, "action.page-missing", "page \"%s\" tile \"%s\" references missing page \"%s\"",
                              page_id, id, target);
                    break;
                }
            }
            ok = check_streams(t, err);
            const cJSON *streams = member(t, "streams");
            bool image = false;
            for (const cJSON *s = streams ? streams->child : NULL; s; s = s->next) {
                image |= stream_type(s) == UI_STREAM_IMAGE;
            }
            if (image) live_images++;
            if (!ok) {
                break;
            }
            const int row = member(t, "row")->valueint;
            const int col = member(t, "col")->valueint;
            const int row_span = member(t, "rowSpan")->valueint;
            const int col_span = member(t, "colSpan")->valueint;
            if (row + row_span > rows || col + col_span > cols) {
                ok = fail(err, "tile.bounds", "page \"%s\" tile \"%s\" exceeds grid bounds %dx%d", page_id, id, rows,
                          cols);
                break;
            }
            for (int r = row; ok && r < row + row_span; r++) {
                for (int c = col; ok && c < col + col_span; c++) {
                    if (owner[r][c] >= 0) {
                        const char *other = member(cJSON_GetArrayItem(member(p, "tiles"), owner[r][c]), "id")->valuestring;
                        ok = fail(err, "tile.overlap", "page \"%s\" tiles \"%s\" and \"%s\" overlap", page_id, other, id);
                    }
                    owner[r][c] = ti;
                }
            }
        }
    }
    free(seen);
    if (ok && live_images > UI_LIVE_IMAGES_MAX) {
        return fail(err, "layout.live_image_limit", "Layout exceeds the live image budget");
    }
    return ok;
}

static size_t text_bytes(const cJSON *v)
{
    return cJSON_IsString(v) ? strlen(v->valuestring) + 1 : 0;
}

static const char *put(char **next, const cJSON *v)
{
    if (!cJSON_IsString(v)) {
        return NULL;
    }
    const size_t n = strlen(v->valuestring) + 1;
    char *s = *next;
    memcpy(s, v->valuestring, n);
    *next += n;
    return s;
}

static size_t align8(size_t n)
{
    return (n + 7) & ~(size_t)7;
}

static td_action_t build_action(const cJSON *a, const cJSON *pages, char **strings)
{
    const cJSON *sink = member(a, "sink");
    td_action_t action = { .type = TD_ACTION_NONE, .hid = sink && strcmp(sink->valuestring, "hid") == 0 };
    const char *type = member(a, "type")->valuestring;
    if (strcmp(type, "key") == 0) {
        action.type = TD_ACTION_KEY;
        action.key.usage = (uint8_t)member(a, "usage")->valueint;
        for (const cJSON *m = member(a, "mods")->child; m != NULL; m = m->next) {
            action.key.mods |= (uint8_t)(1u << modifier_index(m));
        }
    } else if (strcmp(type, "mod") == 0) {
        action.type = TD_ACTION_MOD;
        action.mod.mod = (key_mod_t)modifier_index(member(a, "mod"));
    } else if (strcmp(type, "consumer") == 0) {
        action.type = TD_ACTION_CONSUMER;
        action.consumer.usage = (uint16_t)member(a, "usage")->valueint;
    } else if (strcmp(type, "page") == 0) {
        action.type = TD_ACTION_PAGE;
        action.page.index = (uint8_t)page_index(pages, member(a, "page")->valuestring);
    } else if (strcmp(type, "launch") == 0 || strcmp(type, "open") == 0 || strcmp(type, "command") == 0) {
        action.type = TD_ACTION_APP;
        action.app.kind = type[0] == 'l' ? TD_APP_LAUNCH : type[0] == 'o' ? TD_APP_OPEN : TD_APP_COMMAND;
    } else if (strcmp(type, "widget") == 0) {
        action.type = TD_ACTION_WIDGET;
        action.widget.source = put(strings, member(a, "source"));
        action.widget.field = put(strings, member(a, "field"));
        action.widget.account = put(strings, member(a, "account"));
    }
    return action;
}

/* Copies a checked tree into one arena: the header, the pages, the tiles, then the strings. */
static bool build(const cJSON *root, td_layout_t **out, td_layout_error_t *err)
{
    const cJSON *pages = member(root, "pages");
    size_t page_count = 0;
    size_t tile_count = 0;
    size_t stream_count = 0;
    size_t strings = text_bytes(member(root, "name"));
    for (const cJSON *p = pages->child; p != NULL; p = p->next, page_count++) {
        strings += text_bytes(member(p, "id")) + text_bytes(member(p, "title")) + text_bytes(member(p, "source"));
        const cJSON *tiles = member(p, "tiles");
        for (const cJSON *t = tiles != NULL ? tiles->child : NULL; t != NULL; t = t->next, tile_count++) {
            const cJSON *a = member(t, "action");
            strings += text_bytes(member(t, "id")) + text_bytes(member(t, "label")) + text_bytes(member(t, "icon")) + text_bytes(member(t, "asset"));
            const cJSON *streams = member(t, "streams");
            for (const cJSON *s = streams != NULL ? streams->child : NULL; s != NULL; s = s->next, stream_count++) {
                strings += strlen(s->string) + 1;
            }
            if (strcmp(member(a, "type")->valuestring, "widget") == 0) {
                strings += text_bytes(member(a, "source")) + text_bytes(member(a, "field")) +
                           text_bytes(member(a, "account"));
            }
        }
    }

    const size_t head = align8(sizeof(td_layout_t));
    const size_t page_bytes = align8(page_count * sizeof(ui_page_t));
    const size_t tile_bytes = align8(tile_count * sizeof(ui_tile_t));
    const size_t stream_bytes = align8(stream_count * sizeof(ui_stream_t));
    const size_t total = head + page_bytes + tile_bytes + stream_bytes + strings;
    uint8_t *arena = arena_alloc(total);
    if (arena == NULL) {
        return fail(err, "internal.no-memory", "no memory for a layout of %u bytes", (unsigned)total);
    }
    memset(arena, 0, head + page_bytes + tile_bytes + stream_bytes);
    td_layout_t *layout = (td_layout_t *)arena;
    ui_page_t *page_out = (ui_page_t *)(arena + head);
    ui_tile_t *tile_out = (ui_tile_t *)(arena + head + page_bytes);
    ui_stream_t *stream_out = (ui_stream_t *)(arena + head + page_bytes + tile_bytes);
    char *next = (char *)(arena + head + page_bytes + tile_bytes + stream_bytes);

    layout->schema = (uint8_t)member(root, "schema")->valueint;
    layout->name = put(&next, member(root, "name"));
    layout->orientation = strcmp(member(root, "orientation")->valuestring, "portrait") == 0 ? "portrait" : "landscape";
    layout->full_screen = cJSON_IsTrue(member(root, "fullScreen"));
    layout->pages = page_out;
    layout->page_count = (uint8_t)page_count;
    layout->tile_count = (uint16_t)tile_count;
    layout->arena_bytes = total;

    for (const cJSON *p = pages->child; p != NULL; p = p->next, page_out++) {
        page_out->id = put(&next, member(p, "id"));
        page_out->title = put(&next, member(p, "title"));
        if (is_kind(p, "dashboard")) {
            page_out->kind = UI_PAGE_DASHBOARD;
            page_out->source = put(&next, member(p, "source"));
            page_out->rows = 1;
            page_out->cols = 1;
            continue;
        }
        page_out->kind = UI_PAGE_GRID;
        page_out->rows = (uint8_t)member(p, "rows")->valueint;
        page_out->cols = (uint8_t)member(p, "cols")->valueint;
        page_out->tiles = tile_out;
        for (const cJSON *t = member(p, "tiles")->child; t != NULL; t = t->next, tile_out++) {
            tile_out->id = put(&next, member(t, "id"));
            tile_out->label = put(&next, member(t, "label"));
            tile_out->row = (uint8_t)member(t, "row")->valueint;
            tile_out->col = (uint8_t)member(t, "col")->valueint;
            tile_out->row_span = (uint8_t)member(t, "rowSpan")->valueint;
            tile_out->col_span = (uint8_t)member(t, "colSpan")->valueint;
            tile_out->icon = put(&next, cJSON_IsString(member(t, "asset")) ? member(t, "asset") : member(t, "icon"));
            tile_out->action = build_action(member(t, "action"), pages, &next);
            const cJSON *render = member(t, "render");
            tile_out->render = !cJSON_IsString(render)                   ? UI_RENDER_STATIC
                               : strcmp(render->valuestring, "gif") == 0 ? UI_RENDER_GIF
                               : strcmp(render->valuestring, "bar") == 0 ? UI_RENDER_BAR
                               : strcmp(render->valuestring, "now-playing") == 0 ? UI_RENDER_NOW_PLAYING
                                                                          : UI_RENDER_GRAPH;
            const cJSON *streams = member(t, "streams");
            tile_out->input = cJSON_IsTrue(member(t, "input"));
            tile_out->streams = stream_out;
            for (const cJSON *s = streams != NULL ? streams->child : NULL; s != NULL; s = s->next, stream_out++) {
                const size_t n = strlen(s->string) + 1;
                memcpy(next, s->string, n);
                stream_out->id = next;
                stream_out->type = (ui_stream_type_t)stream_type(s);
                next += n;
                tile_out->stream_count++;
            }
            if (tile_out->icon != NULL && (strncmp(tile_out->icon, "file:", 5) == 0 || strncmp(tile_out->icon, "sha256:", 7) == 0)) {
                layout->icons_wanted++;
            }
            page_out->tile_count++;
        }
    }
    *out = layout;
    return true;
}

bool td_layout_parse(const char *json, size_t len, td_layout_t **out, td_layout_error_t *err)
{
    td_layout_error_t scratch;
    if (err == NULL) {
        err = &scratch;
    }
    memset(err, 0, sizeof *err);
    if (out != NULL) {
        *out = NULL;
    }
    if (json == NULL || out == NULL) {
        return fail(err, "file.json", "no file");
    }
    if (!utf8_valid((const uint8_t *)json, len)) {
        return fail(err, "file.utf8", "file is not well-formed UTF-8");
    }
    if (!scan_text(json, len, err)) {
        return false;
    }
    cJSON *root = cJSON_ParseWithLength(json, len);
    if (root == NULL) {
        return fail(err, "file.json", "cJSON could not hold the file, for want of memory or a number of 64 or more "
                                      "characters");
    }
    const bool ok = check_duplicates(root, err) && check_schema(root, err) && check_semantics(root, err) &&
                    build(root, out, err);
    cJSON_Delete(root);
    if (!ok) {
        sanitize(err);
    }
    return ok;
}
