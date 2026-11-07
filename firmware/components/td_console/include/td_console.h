#pragma once

#include <stdbool.h>
#include <stddef.h>
#include "esp_console.h"
#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

#define TD_REPLY_MAX 512

typedef struct {
    char *buf;
    size_t cap;
    size_t len;
    const char *cmd;
    bool overflow;
} td_reply_t;

void td_reply_begin(td_reply_t *r, char *buf, size_t cap, const char *cmd, bool ok);
void td_reply_str(td_reply_t *r, const char *key, const char *value);
void td_reply_int(td_reply_t *r, const char *key, long long value);
void td_reply_bool(td_reply_t *r, const char *key, bool value);
void td_reply_strs(td_reply_t *r, const char *key, const char *const *values, size_t count);
bool td_reply_end(td_reply_t *r);
bool td_reply_error(char *buf, size_t cap, const char *cmd, const char *message);

/*
 * Adds commands to the console, next to its own, so `help` lists them. The console stores the entries and runs
 * them, nothing more. Call it before td_console_start, which seals the list. A later call fails with
 * ESP_ERR_INVALID_STATE, logs an error and adds nothing. td_cmd_table_add lists the other errors. The strings
 * must stay valid for good.
 */
esp_err_t td_console_register(const esp_console_cmd_t *cmds, size_t count);

#define TD_CONSOLE_MASKS_MAX 4
#define TD_CONSOLE_MASK_WORDS_MAX 30

/*
 * Makes a command's line secret after its first `words` words: the console echoes `*` for every byte from the first
 * byte of word `words` + 1, quotes and escapes included, and the command still receives the real text. `prefix` is
 * the command's leading words one space apart, such as "wifi set", and `words` counts every word of the line up to the
 * secret, so ("wifi set", 3) hides the password of `wifi set <ssid> <password>`. Words split as the console splits
 * them, so quotes and escapes work. Call it before td_console_start. It fails with ESP_ERR_INVALID_STATE after that,
 * ESP_ERR_INVALID_ARG for an empty or badly spaced prefix or a `words` below the prefix's word count or over
 * TD_CONSOLE_MASK_WORDS_MAX, and ESP_ERR_NO_MEM past TD_CONSOLE_MASKS_MAX. `prefix` must stay valid for good.
 */
esp_err_t td_console_mask_after(const char *prefix, size_t words);

/*
 * Sends bytes to the host, and waits for the USB driver's TX ring when it is full, up to a second for each 128
 * bytes, instead of dropping them. Replies go out this way, not through stdio. The stdio path drops a whole write
 * while usb_serial_jtag_is_connected() reads false, which it does for a tick at a time while the display redraws,
 * with the host attached and reading. Holds the stdout lock, so log lines stay whole around the bytes.
 * Returns false when the host did not take every byte.
 */
bool td_console_write(const char *s, size_t len);

/* td_console_write of the line and then CR LF, under one lock, so no log line can land between them. */
bool td_console_write_line(const char *line);

esp_err_t td_console_start(void);

#ifdef __cplusplus
}
#endif
