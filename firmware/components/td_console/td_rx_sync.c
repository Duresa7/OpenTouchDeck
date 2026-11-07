#include "td_rx_sync.h"
#include <string.h>

static bool is_eol(char c)
{
    return c == '\r' || c == '\n';
}

size_t td_rx_sync(td_rx_sync_t *s, char *buf, size_t n, size_t space, size_t *dropped)
{
    size_t from = 0;
    if (s->discarding) {
        while (from < n && !is_eol(buf[from])) {
            from++;
        }
        if (from == n) {
            *dropped += n;
            return 0;
        }
        from++;
        *dropped += from;
        s->discarding = false;
    }

    size_t keep = n - from;
    if (keep == 0) {
        return 0;
    }

    if (space > keep) {
        for (size_t i = 0; i < keep; i++) {
            char c = buf[from + i];
            s->tail = is_eol(c) ? 0 : s->tail + 1;
            buf[i] = c == TD_RX_ABANDON ? '?' : c;
        }
        return keep;
    }

    *dropped += keep + s->tail;
    s->discarding = !is_eol(buf[n - 1]);
    bool abandon = s->tail > 0;
    s->tail = 0;
    if (!abandon) {
        return 0;
    }
    buf[0] = TD_RX_ABANDON;
    return 1;
}

#define FRAME_VERSION 0x01

static void end_frame(td_rx_split_t *s, td_rx_frame_fn frame, void *context)
{
    if (s->mode == TD_RX_FRAME) {
        frame(s->frame, s->frame_len, context);
    } else if (s->mode == TD_RX_ESCAPE || s->mode == TD_RX_FRAME_DISCARD) {
        frame(NULL, 0, context);
    }
}

static void store(td_rx_split_t *s, uint8_t c)
{
    if (s->frame_len == sizeof s->frame) {
        s->mode = TD_RX_FRAME_DISCARD;
    } else {
        s->frame[s->frame_len++] = c;
        s->mode = TD_RX_FRAME;
    }
}

uint32_t td_rx_split_expire(td_rx_split_t *s, uint32_t now_ms, td_rx_frame_fn frame, void *context)
{
    if (s->mode == TD_RX_TEXT) {
        return TD_RX_NO_DEADLINE;
    }
    uint32_t age = now_ms - s->opened_ms;
    if (age < TD_RX_FRAME_DEADLINE_MS) {
        return TD_RX_FRAME_DEADLINE_MS - age;
    }
    end_frame(s, frame, context);
    s->mode = TD_RX_TEXT;
    return TD_RX_NO_DEADLINE;
}

void td_rx_split(td_rx_split_t *s, uint32_t now_ms, char *buf, size_t n,
                 td_rx_text_fn text, td_rx_frame_fn frame, void *context)
{
    td_rx_split_expire(s, now_ms, frame, context);
    size_t text_len = 0;
    for (size_t i = 0; i < n; i++) {
        uint8_t c = (uint8_t)buf[i];
        if (c == 0xc0) {
            if (text_len) {
                text(buf, text_len, context);
                text_len = 0;
            }
            end_frame(s, frame, context);
            s->mode = TD_RX_OPEN;
            s->opened_ms = now_ms;
            s->frame_len = 0;
        } else if (s->mode == TD_RX_TEXT || (s->mode == TD_RX_OPEN && c != FRAME_VERSION)) {
            s->mode = TD_RX_TEXT;
            buf[text_len++] = (char)c;
        } else if (s->mode == TD_RX_ESCAPE) {
            if (c == 0xdc || c == 0xdd) {
                store(s, c == 0xdc ? 0xc0 : 0xdb);
            } else {
                s->mode = TD_RX_FRAME_DISCARD;
            }
        } else if (s->mode != TD_RX_FRAME_DISCARD) {
            if (c == 0xdb) {
                s->mode = TD_RX_ESCAPE;
            } else {
                store(s, c);
            }
        }
    }
    if (text_len) {
        text(buf, text_len, context);
    }
}

static bool has_control(const char *line, size_t len)
{
    for (size_t i = 0; i < len; i++) {
        if ((unsigned char)line[i] < 0x20 && line[i] != '\t') {
            return true;
        }
    }
    return false;
}

/* The split states of esp_console_split_argv. */
enum { WORD_SPACE = 0, WORD_ARG = 1, WORD_QUOTED = 2, WORD_ESCAPE = 8 };

bool td_echo_mask_hides(const td_echo_mask_t *mask, const char *line, size_t len)
{
    if (!mask || !mask->prefix || !*mask->prefix) {
        return false;
    }
    int state = WORD_SPACE;
    size_t words = 0;
    /* Where the current word's text must continue in the prefix, NULL once the word is past the prefix's words or
     * differs from its prefix word. */
    const char *want = mask->prefix;
    bool matched = true;
    for (size_t i = 0; i < len; i++) {
        const char c = line[i];
        int out = -1;
        bool ended = false;
        switch (state) {
        case WORD_SPACE:
            if (c == ' ') {
                break;
            }
            words++;
            if (words > mask->after_words) {
                return matched && want == NULL;
            }
            state = c == '"' ? WORD_QUOTED : c == '\\' ? (WORD_ARG | WORD_ESCAPE) : WORD_ARG;
            out = state == WORD_ARG ? c : -1;
            break;
        case WORD_QUOTED:
            if (c == '"') {
                ended = true;
            } else if (c == '\\') {
                state = WORD_QUOTED | WORD_ESCAPE;
            } else {
                out = c;
            }
            break;
        case WORD_ARG | WORD_ESCAPE:
        case WORD_QUOTED | WORD_ESCAPE:
            out = c == '\\' || c == '"' || c == ' ' ? c : -1;
            state &= ~WORD_ESCAPE;
            break;
        default:
            if (c == ' ') {
                ended = true;
            } else if (c == '\\') {
                state = WORD_ARG | WORD_ESCAPE;
            } else {
                out = c;
            }
            break;
        }
        if (out >= 0 && want != NULL) {
            if (*want == (char)out && *want != ' ' && *want != '\0') {
                want++;
            } else {
                matched = false;
            }
        }
        if (ended) {
            state = WORD_SPACE;
            if (want != NULL) {
                /* The word ends where its prefix word ends, or it is not that word. */
                if (*want == ' ') {
                    want++;
                } else if (*want == '\0') {
                    want = NULL;
                } else {
                    matched = false;
                }
            }
        }
    }
    return false;
}

static void line_reset(td_line_t *s)
{
    *s = (td_line_t){ .masks = s->masks, .mask_count = s->mask_count };
}

static bool line_secret(const td_line_t *s, const char *line)
{
    for (size_t i = 0; i < s->mask_count; i++) {
        if (td_echo_mask_hides(&s->masks[i], line, s->len)) {
            return true;
        }
    }
    return false;
}

td_line_status_t td_line_feed(td_line_t *s, int c, bool dropped,
                              char *line, size_t cap, char echo[3], size_t *echo_len)
{
    *echo_len = 0;
    if (c == TD_RX_ABANDON || (c < 0 && s->len == 0 && dropped)) {
        memcpy(echo, "\r\n", 2);
        *echo_len = 2;
        line_reset(s);
        line[0] = '\0';
        return TD_LINE_RESTART;
    }
    if (c < 0) {
        return TD_LINE_MORE;
    }
    if (c == '\r' || c == '\n') {
        memcpy(echo, "\r\n", 2);
        *echo_len = 2;
        line[s->len] = '\0';
        td_line_status_t status = s->too_long ? TD_LINE_TOO_LONG
                                : has_control(line, s->len) ? TD_LINE_CONTROL : TD_LINE_OK;
        line_reset(s);
        return status;
    }
    if (c == '\b' || c == 0x7f) {
        if (s->len > 0 && !s->too_long) {
            s->len--;
            if (s->len < s->secret_from) {
                s->secret_from = 0;
            }
            memcpy(echo, "\b \b", 3);
            *echo_len = 3;
        }
    } else if (s->len + 1 < cap) {
        line[s->len++] = (char)c;
        if (s->secret_from == 0 && line_secret(s, line)) {
            s->secret_from = s->len;
        }
        echo[0] = s->secret_from != 0 ? '*' : (char)c;
        *echo_len = 1;
    } else {
        s->too_long = true;
    }
    return TD_LINE_MORE;
}
