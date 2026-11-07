#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Queued after the bytes of a line that a drop cut, so the reader abandons its partial line. */
#define TD_RX_ABANDON '\0'

#define TD_RX_FRAME_MAX 1100
#define TD_RX_FRAME_DEADLINE_MS 500
#define TD_RX_NO_DEADLINE UINT32_MAX

typedef void (*td_rx_text_fn)(char *text, size_t len, void *context);
typedef void (*td_rx_frame_fn)(const uint8_t *body, size_t len, void *context);

typedef enum {
    TD_RX_TEXT,
    TD_RX_OPEN,
    TD_RX_FRAME,
    TD_RX_ESCAPE,
    TD_RX_FRAME_DISCARD,
} td_rx_mode_t;

typedef struct {
    size_t tail;
    bool discarding;
} td_rx_sync_t;

typedef struct {
    td_rx_mode_t mode;
    uint32_t opened_ms;
    size_t frame_len;
    uint8_t frame[TD_RX_FRAME_MAX];
} td_rx_split_t;

/*
 * Emits text and unescaped frame bodies in wire order for n bytes read at
 * now_ms. Every delimiter ends the frame in progress and opens the next one.
 * Only a ver byte of 1 after it starts a body. Any other byte means no frame
 * opened, and it goes to text. Every ended body goes to frame, decoded or not,
 * and NULL reports one that broke SLIP or overflowed.
 */
void td_rx_split(td_rx_split_t *s, uint32_t now_ms, char *buf, size_t n,
                 td_rx_text_fn text, td_rx_frame_fn frame, void *context);

/*
 * Ends a frame opened TD_RX_FRAME_DEADLINE_MS or more before now_ms, so the
 * bytes after it are text. Returns the milliseconds left before the open
 * frame's deadline, or TD_RX_NO_DEADLINE when no frame is open.
 */
uint32_t td_rx_split_expire(td_rx_split_t *s, uint32_t now_ms, td_rx_frame_fn frame, void *context);

/* Register before td_console_start. NULL bodies report malformed SLIP. */
void td_console_set_frame_handler(td_rx_frame_fn handler, void *context);

/*
 * Decides what the input task queues for one read of n bytes in buf, given the
 * free bytes in the queue. On return buf holds the m bytes to queue, which is
 * the return value, and *dropped has grown by the host bytes that are lost.
 *
 * A read is queued whole when one byte stays free for TD_RX_ABANDON. Otherwise
 * it is dropped, and so is every byte through the next CR or LF, so no line
 * that spans a drop reaches the reader. The part of that line queued earlier is
 * counted as dropped and cancelled by TD_RX_ABANDON.
 *
 * A NUL from the host is queued as '?', because NUL is the abandon marker.
 */
size_t td_rx_sync(td_rx_sync_t *s, char *buf, size_t n, size_t space, size_t *dropped);

/*
 * A console line that starts with the words of `prefix`, one space apart, is secret from word after_words + 1 on: the
 * echo shows `*` for each of its bytes, quotes and escapes included, while the line itself keeps the real text.
 * after_words counts the prefix's own words, so {"wifi set", 3} hides the password of `wifi set <ssid> <password>`.
 * Words split the way esp_console_split_argv splits them: on spaces, with `"` quoting and `\` escaping.
 */
typedef struct {
    const char *prefix;
    size_t after_words;
} td_echo_mask_t;

/* True when `line`, `len` bytes, has started word after_words + 1 of a line that `mask` hides. */
bool td_echo_mask_hides(const td_echo_mask_t *mask, const char *line, size_t len);

typedef struct {
    size_t len;
    bool too_long;
    /* The line length at the first secret byte, 0 while nothing is secret. */
    size_t secret_from;
    const td_echo_mask_t *masks;
    size_t mask_count;
} td_line_t;

typedef enum {
    TD_LINE_MORE,
    TD_LINE_OK,
    TD_LINE_TOO_LONG,
    TD_LINE_CONTROL,
    TD_LINE_RESTART,
} td_line_status_t;

/*
 * c == -1 is an idle read. echo holds at most three bytes, already in wire form.
 * A line that keeps a control byte other than tab ends as TD_LINE_CONTROL.
 * A byte of a line that one of s->masks hides echoes as `*`. A new line keeps the masks.
 */
td_line_status_t td_line_feed(td_line_t *s, int c, bool dropped,
                              char *line, size_t cap, char echo[3], size_t *echo_len);

#ifdef __cplusplus
}
#endif
