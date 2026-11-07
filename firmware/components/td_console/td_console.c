#include "td_console.h"
#include "td_cmd_table.h"
#include "td_rx_sync.h"
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include "esp_app_desc.h"
#include "esp_check.h"
#include "esp_console.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_stdio.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "driver/usb_serial_jtag.h"
#include "freertos/FreeRTOS.h"
#include "freertos/stream_buffer.h"
#include "freertos/task.h"

#define TD_PROMPT "td> "
#define TD_LINE_MAX 256
#define TD_WORD_MAX 64
#define TD_TASK_STACK 6144
#define TD_TASK_PRIORITY 2
#define TD_RX_TASK_STACK 2048
#define TD_RX_TASK_PRIORITY 10
#define TD_RX_CHUNK 64
#define TD_RX_QUEUE_BYTES 2048
#define TD_RX_IDLE_MS 100
#define TD_TX_DRAIN_MS 250

static const char *TAG = "td_console";

/* The driver's TX ring holds 256 bytes, and a single write larger than the ring never fits. */
#define TD_TX_CHUNK 128
#define TD_TX_WAIT_MS 1000

static bool write_locked(const char *s, size_t len)
{
    size_t done = 0;
    while (done < len) {
        const size_t n = len - done < TD_TX_CHUNK ? len - done : TD_TX_CHUNK;
        if (usb_serial_jtag_write_bytes(s + done, n, pdMS_TO_TICKS(TD_TX_WAIT_MS)) != (int)n) {
            break;
        }
        done += n;
    }
    return done == len;
}

bool td_console_write(const char *s, size_t len)
{
    flockfile(stdout);
    fflush(stdout);
    const bool ok = write_locked(s, len);
    funlockfile(stdout);
    return ok;
}

bool td_console_write_line(const char *line)
{
    flockfile(stdout);
    fflush(stdout);
    const bool ok = write_locked(line, strlen(line)) && write_locked("\r\n", 2);
    funlockfile(stdout);
    return ok;
}

static void emit(td_reply_t *r)
{
    td_reply_end(r);
    td_console_write_line(r->buf);
}

static void emit_error(const char *cmd, const char *message)
{
    char buf[TD_REPLY_MAX];
    td_reply_error(buf, sizeof buf, cmd, message);
    td_console_write_line(buf);
}

static int cmd_help(int argc, char **argv);

static int cmd_version(int argc, char **argv)
{
    const esp_app_desc_t *app = esp_app_get_description();
    char sha[2 * sizeof app->app_elf_sha256 + 1];
    for (size_t i = 0; i < sizeof app->app_elf_sha256; i++) {
        snprintf(sha + 2 * i, 3, "%02x", app->app_elf_sha256[i]);
    }

    char buf[TD_REPLY_MAX];
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "version", true);
    td_reply_str(&r, "project_name", app->project_name);
    td_reply_str(&r, "version", app->version);
    td_reply_str(&r, "idf_ver", app->idf_ver);
    td_reply_str(&r, "date", app->date);
    td_reply_str(&r, "time", app->time);
    td_reply_str(&r, "elf_sha256", sha);
    emit(&r);
    return 0;
}

static int cmd_heap(int argc, char **argv)
{
    char buf[TD_REPLY_MAX];
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "heap", true);
    td_reply_int(&r, "free_heap", esp_get_free_heap_size());
    td_reply_int(&r, "min_free_heap", esp_get_minimum_free_heap_size());
    td_reply_int(&r, "free_internal", heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT));
    td_reply_int(&r, "min_free_internal", heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT));
    td_reply_int(&r, "free_psram", heap_caps_get_free_size(MALLOC_CAP_SPIRAM));
    td_reply_int(&r, "min_free_psram", heap_caps_get_minimum_free_size(MALLOC_CAP_SPIRAM));
    emit(&r);
    return 0;
}

static int cmd_uptime(int argc, char **argv)
{
    char buf[TD_REPLY_MAX];
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "uptime", true);
    td_reply_int(&r, "uptime_ms", esp_timer_get_time() / 1000);
    emit(&r);
    return 0;
}

static int cmd_reboot(int argc, char **argv)
{
    char buf[TD_REPLY_MAX];
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "reboot", true);
    emit(&r);
    fflush(stdout);
    usb_serial_jtag_wait_tx_done(pdMS_TO_TICKS(TD_TX_DRAIN_MS));
    esp_restart();
    return 0;
}

static const esp_console_cmd_t s_builtin_cmds[] = {
    { .command = "help", .help = "list the console commands", .func = cmd_help },
    { .command = "version", .help = "report the app descriptor", .func = cmd_version },
    { .command = "heap", .help = "report free heap", .func = cmd_heap },
    { .command = "uptime", .help = "report milliseconds since boot", .func = cmd_uptime },
    { .command = "reboot", .help = "restart the board", .func = cmd_reboot },
};

static td_cmd_table_t s_table;
static td_echo_mask_t s_masks[TD_CONSOLE_MASKS_MAX];
static size_t s_mask_count;

esp_err_t td_console_mask_after(const char *prefix, size_t words)
{
    if (s_table.sealed) {
        return ESP_ERR_INVALID_STATE;
    }
    size_t prefix_words = 0;
    for (const char *p = prefix; p && *p; p++) {
        prefix_words += p == prefix || p[-1] == ' ';
        if (*p == ' ' && (p == prefix || p[-1] == ' ' || p[1] == '\0')) {
            return ESP_ERR_INVALID_ARG;
        }
    }
    if (prefix_words == 0 || words < prefix_words || words > TD_CONSOLE_MASK_WORDS_MAX) {
        return ESP_ERR_INVALID_ARG;
    }
    if (s_mask_count == TD_CONSOLE_MASKS_MAX) {
        return ESP_ERR_NO_MEM;
    }
    s_masks[s_mask_count++] = (td_echo_mask_t){ .prefix = prefix, .after_words = words };
    return ESP_OK;
}

/* The table starts with the console's own commands. `help` lists them first, and no other component can reuse
 * their names. */
static td_cmd_table_t *table(void)
{
    if (s_table.count == 0) {
        td_cmd_table_add(&s_table, s_builtin_cmds, sizeof s_builtin_cmds / sizeof s_builtin_cmds[0]);
    }
    return &s_table;
}

static int cmd_help(int argc, char **argv)
{
    const char *names[TD_CMD_TABLE_MAX];
    const size_t count = td_cmd_table_names(table(), names);

    char buf[TD_REPLY_MAX];
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "help", true);
    td_reply_strs(&r, "commands", names, count);
    emit(&r);
    return 0;
}

esp_err_t td_console_register(const esp_console_cmd_t *cmds, size_t count)
{
    const esp_err_t err = td_cmd_table_add(table(), cmds, count);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "registering %u command(s) starting with \"%s\" failed: %s%s", (unsigned)count,
                 count > 0 && cmds[0].command ? cmds[0].command : "", esp_err_to_name(err),
                 err == ESP_ERR_INVALID_STATE ? " (register before td_console_start)" : "");
    }
    return err;
}

typedef struct {
    char in[64];
    size_t pos;
    size_t len;
    char echo[3 * 64];
    size_t echo_len;
} rx_t;

static void echo_flush(rx_t *rx)
{
    if (rx->echo_len > 0) {
        td_console_write(rx->echo, rx->echo_len);
        rx->echo_len = 0;
    }
}

static void echo_add(rx_t *rx, const char *s, size_t n)
{
    memcpy(rx->echo + rx->echo_len, s, n);
    rx->echo_len += n;
}

static StreamBufferHandle_t s_rx_queue;
static atomic_uint s_rx_dropped;
static td_rx_sync_t s_rx_sync;
static td_rx_split_t s_rx_split;
static td_rx_frame_fn s_frame_handler;
static void *s_frame_context;

void td_console_set_frame_handler(td_rx_frame_fn handler, void *context)
{
    s_frame_handler = handler;
    s_frame_context = context;
}

static void receive_text(char *text, size_t len, void *context)
{
    (void)context;
    size_t lost = 0;
    size_t count = td_rx_sync(&s_rx_sync, text, len, xStreamBufferSpacesAvailable(s_rx_queue), &lost);
    if (count > 0) {
        size_t sent = xStreamBufferSend(s_rx_queue, text, count, 0);
        lost += count - sent;
    }
    if (lost > 0) {
        atomic_fetch_add(&s_rx_dropped, (unsigned)lost);
    }
}

static void receive_frame(const uint8_t *body, size_t len, void *context)
{
    (void)context;
    if (s_frame_handler) {
        s_frame_handler(body, len, s_frame_context);
    }
}

static uint32_t now_ms(void)
{
    return (uint32_t)(esp_timer_get_time() / 1000);
}

/*
 * The driver's RX interrupt copies each USB packet into a 256-byte ring and
 * discards the whole packet, without a trace, when the ring is full. Printing
 * costs the console task about 13 us per output byte, so a 220-byte reply
 * keeps it from reading for about 3 ms and a burst of short lines fills that
 * ring. This task outranks the console and drains the ring in reads of up to 64
 * bytes, handing frames to the link and text to s_rx_queue. When the queue is
 * full, td_rx_sync drops the read and the rest of the line it cuts, and the
 * loss is counted and reported instead of hidden. It reads the driver directly
 * because read() on stdin takes one byte per ring operation, which is slower
 * than back-to-back 64-byte packets.
 */
static void rx_task(void *arg)
{
    char chunk[TD_RX_CHUNK];
    for (;;) {
        uint32_t wait = td_rx_split_expire(&s_rx_split, now_ms(), receive_frame, NULL);
        int n = usb_serial_jtag_read_bytes(chunk, sizeof chunk,
                                           wait == TD_RX_NO_DEADLINE ? portMAX_DELAY : pdMS_TO_TICKS(wait));
        if (n > 0) {
            td_rx_split(&s_rx_split, now_ms(), chunk, (size_t)n, receive_text, receive_frame, NULL);
        } else if (n < 0 || wait == TD_RX_NO_DEADLINE) {
            vTaskDelay(pdMS_TO_TICKS(10));
        }
    }
}

static void report_dropped_input(void)
{
    unsigned dropped = atomic_exchange(&s_rx_dropped, 0);
    if (dropped == 0) {
        return;
    }
    char buf[TD_REPLY_MAX];
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "console", false);
    td_reply_str(&r, "error", "input overflow");
    td_reply_int(&r, "dropped_bytes", dropped);
    emit(&r);
}

static int next_char(rx_t *rx)
{
    while (rx->pos == rx->len) {
        echo_flush(rx);
        rx->len = xStreamBufferReceive(s_rx_queue, rx->in, sizeof rx->in, pdMS_TO_TICKS(TD_RX_IDLE_MS));
        rx->pos = 0;
        if (rx->len == 0) {
            return -1;
        }
    }
    return (unsigned char)rx->in[rx->pos++];
}

static td_line_status_t read_line(rx_t *rx, char *line, size_t cap)
{
    td_line_t state = { .masks = s_masks, .mask_count = s_mask_count };
    for (;;) {
        int c = next_char(rx);
        char echo[3];
        size_t echo_len;
        td_line_status_t status = td_line_feed(&state, c, atomic_load(&s_rx_dropped) > 0,
                                               line, cap, echo, &echo_len);
        echo_add(rx, echo, echo_len);
        if (status != TD_LINE_MORE) {
            echo_flush(rx);
            return status;
        }
    }
}

/* A wipe the compiler can't drop: every store goes through a volatile pointer. A line may hold a password
 * (`wifi set <ssid> <password>`), so nothing keeps it after it ran. */
static void wipe(void *p, size_t n)
{
    volatile unsigned char *b = p;
    while (n--) {
        *b++ = 0;
    }
}

static void command_word(const char *line, char *word, size_t cap)
{
    char copy[TD_LINE_MAX];
    char *argv[2];
    strlcpy(copy, line, sizeof copy);
    word[0] = '\0';
    if (esp_console_split_argv(copy, argv, 2) > 0) {
        strlcpy(word, argv[0], cap);
    }
    wipe(copy, sizeof copy);
}

static void run_line(const char *line)
{
    int ret;
    esp_err_t err = esp_console_run(line, &ret);
    if (err == ESP_OK || err == ESP_ERR_INVALID_ARG) {
        return;
    }
    char word[TD_WORD_MAX];
    command_word(line, word, sizeof word);
    emit_error(word, err == ESP_ERR_NOT_FOUND ? "unknown command" : "internal error");
}

static void console_task(void *arg)
{
    rx_t rx = { 0 };
    char line[TD_LINE_MAX];
    for (;;) {
        report_dropped_input();
        td_console_write(TD_PROMPT, sizeof(TD_PROMPT) - 1);
        const td_line_status_t status = read_line(&rx, line, sizeof line);
        switch (status) {
        case TD_LINE_OK:
            run_line(line);
            break;
        case TD_LINE_TOO_LONG:
        case TD_LINE_CONTROL: {
            char word[TD_WORD_MAX];
            command_word(line, word, sizeof word);
            emit_error(word, status == TD_LINE_TOO_LONG ? "line too long" : "control character");
            break;
        }
        case TD_LINE_RESTART:
        case TD_LINE_MORE:
            break;
        }
        /* The line and the bytes it came in are done with: wipe both, but keep the unread bytes of the next line. */
        wipe(line, sizeof line);
        wipe(rx.in, rx.pos);
        wipe(rx.in + rx.len, sizeof rx.in - rx.len);
    }
}

esp_err_t td_console_start(void)
{
    td_cmd_table_seal(table());
    ESP_RETURN_ON_ERROR(esp_stdio_install_io_driver(), TAG, "stdio driver");
    if (!usb_serial_jtag_is_driver_installed()) {
        ESP_LOGW(TAG, "console is off: its input task reads the USB-Serial/JTAG driver, which only the USB-Serial/JTAG primary console installs");
        return ESP_ERR_NOT_SUPPORTED;
    }

    esp_console_config_t config = ESP_CONSOLE_CONFIG_DEFAULT();
    config.max_cmdline_length = TD_LINE_MAX;
    ESP_RETURN_ON_ERROR(esp_console_init(&config), TAG, "console init");
    for (size_t i = 0; i < s_table.count; i++) {
        ESP_RETURN_ON_ERROR(esp_console_cmd_register(&s_table.cmds[i]), TAG, "register %s", s_table.cmds[i].command);
    }

    s_rx_queue = xStreamBufferCreate(TD_RX_QUEUE_BYTES, 1);
    ESP_RETURN_ON_FALSE(s_rx_queue, ESP_ERR_NO_MEM, TAG, "rx queue");
    ESP_RETURN_ON_FALSE(xTaskCreate(rx_task, "td_rx", TD_RX_TASK_STACK, NULL, TD_RX_TASK_PRIORITY, NULL) == pdPASS,
                        ESP_FAIL, TAG, "rx task");
    ESP_RETURN_ON_FALSE(xTaskCreate(console_task, "td_console", TD_TASK_STACK, NULL, TD_TASK_PRIORITY, NULL) == pdPASS,
                        ESP_FAIL, TAG, "console task");
    return ESP_OK;
}
