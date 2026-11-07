/*
 * The radio build's Wi-Fi station on the host: td_wifi.c (included, so a test can reboot it), the policy, the NVS store,
 * the `wifi.*` messages and the `wifi` console command run unchanged over a fake NVS and a fake driver. The console's
 * echo mask (td_rx_sync.c) runs here too.
 *
 * Every SSID and password here is made up. The checks look for the password in every reply, console line and log line.
 */
#include "td_wifi.c"

#include <stdarg.h>
#include <stdlib.h>
#include "cJSON.h"
#include "nvs.h"
#include "nvs_flash.h"
#include "td_console.h"
#include "td_link_wifi.h"
#include "td_rx_sync.h"

/* ---- Checks ---- */

static int g_failures;

#define CHECK(cond)                                                                                   \
    do {                                                                                              \
        if (!(cond)) {                                                                                \
            fprintf(stderr, "%s:%d: %s: failed: %s\n", __FILE__, __LINE__, __func__, #cond);          \
            g_failures++;                                                                             \
        }                                                                                             \
    } while (0)

/* ---- Host stand-ins: errors, log, FreeRTOS ---- */

const char *esp_err_to_name(esp_err_t err)
{
    switch (err) {
    case ESP_OK: return "ESP_OK";
    case ESP_ERR_NOT_FOUND: return "ESP_ERR_NOT_FOUND";
    case ESP_ERR_INVALID_ARG: return "ESP_ERR_INVALID_ARG";
    case ESP_ERR_NVS_NOT_ENOUGH_SPACE: return "ESP_ERR_NVS_NOT_ENOUGH_SPACE";
    default: return "ESP_ERR";
    }
}

static char g_log[16384];
static size_t g_log_len;

void host_log(const char *level, const char *tag, const char *format, ...)
{
    char line[512];
    va_list args;
    va_start(args, format);
    vsnprintf(line, sizeof line, format, args);
    va_end(args);
    g_log_len += (size_t)snprintf(g_log + g_log_len, sizeof g_log - g_log_len, "%s %s: %s\n", level, tag, line);
    assert(g_log_len < sizeof g_log);
}

struct host_mutex {
    bool held;
};

SemaphoreHandle_t xSemaphoreCreateMutex(void)
{
    return calloc(1, sizeof(struct host_mutex));
}

BaseType_t xSemaphoreTake(SemaphoreHandle_t m, TickType_t wait)
{
    (void)wait;
    assert(!m->held);
    m->held = true;
    return pdTRUE;
}

BaseType_t xSemaphoreGive(SemaphoreHandle_t m)
{
    assert(m->held);
    m->held = false;
    return pdTRUE;
}

void vSemaphoreDelete(SemaphoreHandle_t m)
{
    free(m);
}

/* ---- The fake NVS: one namespace, one key, kept across a reboot ---- */

static struct {
    bool has;
    unsigned char blob[256];
    size_t len;
    /* Errors to inject. */
    esp_err_t init_err, set_err, commit_err;
    int open_handles;
    int writes;
} g_nvs;

esp_err_t nvs_flash_init(void)
{
    return g_nvs.init_err;
}

esp_err_t nvs_open(const char *name, nvs_open_mode_t mode, nvs_handle_t *handle)
{
    CHECK(!strcmp(name, "td_wifi"));
    if (mode == NVS_READONLY && !g_nvs.has && !g_nvs.writes) {
        return ESP_ERR_NVS_NOT_FOUND;
    }
    g_nvs.open_handles++;
    *handle = 7;
    return ESP_OK;
}

esp_err_t nvs_get_blob(nvs_handle_t handle, const char *key, void *out, size_t *length)
{
    CHECK(handle == 7 && !strcmp(key, "net"));
    if (!g_nvs.has) {
        return ESP_ERR_NVS_NOT_FOUND;
    }
    if (*length < g_nvs.len) {
        return ESP_ERR_NVS_INVALID_LENGTH;
    }
    memcpy(out, g_nvs.blob, g_nvs.len);
    *length = g_nvs.len;
    return ESP_OK;
}

esp_err_t nvs_set_blob(nvs_handle_t handle, const char *key, const void *value, size_t length)
{
    CHECK(handle == 7 && !strcmp(key, "net"));
    if (g_nvs.set_err != ESP_OK) {
        return g_nvs.set_err;
    }
    assert(length <= sizeof g_nvs.blob);
    memcpy(g_nvs.blob, value, length);
    g_nvs.len = length;
    g_nvs.has = true;
    g_nvs.writes++;
    return ESP_OK;
}

esp_err_t nvs_erase_key(nvs_handle_t handle, const char *key)
{
    CHECK(handle == 7 && !strcmp(key, "net"));
    if (!g_nvs.has) {
        return ESP_ERR_NVS_NOT_FOUND;
    }
    g_nvs.has = false;
    g_nvs.len = 0;
    g_nvs.writes++;
    return ESP_OK;
}

esp_err_t nvs_commit(nvs_handle_t handle)
{
    CHECK(handle == 7);
    return g_nvs.commit_err;
}

void nvs_close(nvs_handle_t handle)
{
    CHECK(handle == 7);
    g_nvs.open_handles--;
}

/* ---- The fake driver ---- */

static struct {
    bool refuse;
    int joins, leaves, stops;
    char ssid[64], password[80];
    uint32_t retry_ms, dhcp_ms;
    int rssi;
} g_drv;

static bool drv_join(const char *ssid, const char *password)
{
    g_drv.joins++;
    snprintf(g_drv.ssid, sizeof g_drv.ssid, "%s", ssid);
    snprintf(g_drv.password, sizeof g_drv.password, "%s", password);
    return !g_drv.refuse;
}

static void drv_leave(void) { g_drv.leaves++; }
static void drv_retry_in(uint32_t ms) { g_drv.retry_ms = ms; }
static void drv_dhcp_in(uint32_t ms) { g_drv.dhcp_ms = ms; }

static void drv_stop_timers(void)
{
    g_drv.stops++;
    g_drv.retry_ms = 0;
    g_drv.dhcp_ms = 0;
}

static bool drv_rssi(int *dbm)
{
    *dbm = g_drv.rssi;
    return true;
}

static const td_wifi_driver_t g_driver = {
    .join = drv_join,
    .leave = drv_leave,
    .retry_in = drv_retry_in,
    .dhcp_in = drv_dhcp_in,
    .stop_timers = drv_stop_timers,
    .rssi = drv_rssi,
};

/* ---- The link's and the console's side, as td_link_msgs.c and td_console.c do it ---- */

void td_link_wifi_fail(cJSON *reply, const char *reason, const char *error)
{
    cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
    cJSON_DeleteItemFromObjectCaseSensitive(reply, "error");
    cJSON_AddStringToObject(reply, "error", error);
    cJSON_AddStringToObject(reply, "reason", reason);
}

bool td_link_wifi_fields(const cJSON *request, const char *const *allowed, size_t count)
{
    const cJSON *a;
    cJSON_ArrayForEach(a, request) {
        bool known = !strcmp(a->string, "m");
        for (size_t i = 0; i < count && !known; i++) {
            known = !strcmp(a->string, allowed[i]);
        }
        if (!known) return false;
        for (const cJSON *b = a->next; b; b = b->next) {
            if (!strcmp(a->string, b->string)) return false;
        }
    }
    return true;
}

static char g_console[4096];
static size_t g_console_len;

bool td_console_write_line(const char *line)
{
    g_console_len += (size_t)snprintf(g_console + g_console_len, sizeof g_console - g_console_len, "%s\n", line);
    assert(g_console_len < sizeof g_console);
    return true;
}

bool td_reply_error(char *buf, size_t cap, const char *cmd, const char *message)
{
    return snprintf(buf, cap, "@td {\"cmd\":\"%s\",\"ok\":false,\"error\":\"%s\"}", cmd, message) < (int)cap;
}

/* ---- Helpers ---- */

/* Made-up networks. None of them exists. */
#define SSID "td-test-nonexistent"
#define PASSWORD "dummy-pass-1234"
#define SSID2 "td-test-other"
#define PASSWORD2 "other-dummy-5678"

/* A cold boot: the station's RAM is gone, the NVS blob stays. */
static void reboot(void)
{
    if (s_lock) {
        vSemaphoreDelete(s_lock);
    }
    s_lock = NULL;
    s_driver = NULL;
    memset(&s_state, 0, sizeof s_state);
    memset(s_password, 0, sizeof s_password);
    s_status = TD_WIFI_OFF;
    s_started = false;
    s_associated = false;
    s_attempt = 0;
    memset(&g_drv, 0, sizeof g_drv);
    g_drv.rssi = -52;
}

/* A factory-fresh Device with an empty NVS, booted and with its station started. */
static void fresh(bool started)
{
    memset(&g_nvs, 0, sizeof g_nvs);
    reboot();
    CHECK(td_wifi_init(&g_driver) == ESP_OK);
    if (started) {
        td_wifi_started(ESP_OK);
    }
}

static const uint8_t *ssid_bytes(const char *ssid)
{
    return (const uint8_t *)ssid;
}

/* Runs one `wifi.*` message through the handler the link calls, and returns the reply. */
static cJSON *message(const char *json)
{
    cJSON *request = cJSON_Parse(json);
    assert(request);
    const char *name = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "m"));
    cJSON *reply = cJSON_CreateObject();
    cJSON_AddStringToObject(reply, "m", name);
    cJSON_AddBoolToObject(reply, "ok", true);
    td_wifi_link_message(name, request, reply);
    /* The handler wipes the password the request carried before the link frees it. */
    const cJSON *password = cJSON_GetObjectItemCaseSensitive(request, "password");
    if (cJSON_IsString(password)) {
        CHECK(password->valuestring[0] == '\0');
    }
    cJSON_Delete(request);
    return reply;
}

static bool reply_ok(const cJSON *reply)
{
    return cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "ok"));
}

static const char *reply_str(const cJSON *reply, const char *key)
{
    return cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(reply, key));
}

static bool reply_null(const cJSON *reply, const char *key)
{
    return cJSON_IsNull(cJSON_GetObjectItemCaseSensitive(reply, key));
}

/* The reply, printed, never holds a password. */
static void check_reply_secret_free(const cJSON *reply)
{
    char *text = cJSON_PrintUnformatted(reply);
    CHECK(strstr(text, PASSWORD) == NULL && strstr(text, PASSWORD2) == NULL);
    CHECK(cJSON_GetObjectItemCaseSensitive(reply, "password") == NULL);
    free(text);
}

/* Runs the `wifi` console command with these words. */
static void console(int argc, const char *const *words)
{
    size_t count = 0;
    const esp_console_cmd_t *cmds = td_wifi_console_commands(&count);
    CHECK(count == 1 && !strcmp(cmds[0].command, "wifi"));
    char storage[8][80];
    char *argv[8];
    for (int i = 0; i < argc; i++) {
        snprintf(storage[i], sizeof storage[i], "%s", words[i]);
        argv[i] = storage[i];
    }
    CHECK(cmds[0].func(argc, argv) == 0);
    /* The command wipes the password it was given. */
    if (argc >= 4) {
        CHECK(argv[3][0] == '\0');
    }
}

/* ---- Policy ---- */

static void test_credentials(void)
{
    CHECK(td_wifi_credentials_valid(SSID, PASSWORD));
    CHECK(td_wifi_credentials_valid(SSID, ""));
    CHECK(td_wifi_credentials_valid("x", "12345678"));
    CHECK(td_wifi_credentials_valid("12345678901234567890123456789012", "12345678"));
    CHECK(!td_wifi_credentials_valid("123456789012345678901234567890123", "12345678"));
    CHECK(!td_wifi_credentials_valid("", PASSWORD));
    CHECK(!td_wifi_credentials_valid(NULL, PASSWORD));
    CHECK(!td_wifi_credentials_valid(SSID, NULL));
    CHECK(!td_wifi_credentials_valid(SSID, "1234567"));
    char long_password[65];
    memset(long_password, 'a', 63);
    long_password[63] = '\0';
    CHECK(td_wifi_credentials_valid(SSID, long_password));
    /* 64 bytes are a raw key: hex digits only. */
    memset(long_password, 'g', 64);
    long_password[64] = '\0';
    CHECK(!td_wifi_credentials_valid(SSID, long_password));
    memset(long_password, 'F', 64);
    CHECK(td_wifi_credentials_valid(SSID, long_password));
    CHECK(!td_wifi_credentials_valid("tab\there", PASSWORD));
    CHECK(!td_wifi_credentials_valid(SSID, "line\nbreak1"));
    CHECK(td_wifi_credentials_valid("caf\xc3\xa9 net", PASSWORD));
    CHECK(!td_wifi_credentials_valid("bad \xc3", PASSWORD));
    CHECK(!td_wifi_credentials_valid("overlong \xc0\xaf", PASSWORD));
    CHECK(!td_wifi_credentials_valid("surrogate \xed\xa0\x80", PASSWORD));
}

static void test_reasons_and_backoff(void)
{
    CHECK(td_wifi_reason_from_driver(201) == TD_WIFI_REASON_NOT_FOUND);
    CHECK(td_wifi_reason_from_driver(212) == TD_WIFI_REASON_NOT_FOUND);
    CHECK(td_wifi_reason_from_driver(202) == TD_WIFI_REASON_WRONG_PASSWORD);
    CHECK(td_wifi_reason_from_driver(15) == TD_WIFI_REASON_WRONG_PASSWORD);
    CHECK(td_wifi_reason_from_driver(204) == TD_WIFI_REASON_WRONG_PASSWORD);
    CHECK(td_wifi_reason_from_driver(210) == TD_WIFI_REASON_SECURITY);
    CHECK(td_wifi_reason_from_driver(203) == TD_WIFI_REASON_REFUSED);
    CHECK(td_wifi_reason_from_driver(200) == TD_WIFI_REASON_LOST);
    CHECK(td_wifi_reason_from_driver(205) == TD_WIFI_REASON_DRIVER);
    CHECK(td_wifi_reason_from_driver(1) == TD_WIFI_REASON_OTHER);
    CHECK(td_wifi_reason_name(TD_WIFI_REASON_NONE) == NULL);
    CHECK(!strcmp(td_wifi_reason_name(TD_WIFI_REASON_NOT_FOUND), "not-found"));
    CHECK(!strcmp(td_wifi_reason_name(TD_WIFI_REASON_WRONG_PASSWORD), "wrong-password"));
    CHECK(!strcmp(td_wifi_reason_name(TD_WIFI_REASON_NO_IP), "no-ip"));
    CHECK(!strcmp(td_wifi_status_name(TD_WIFI_OFF), "off"));
    CHECK(!strcmp(td_wifi_status_name(TD_WIFI_FAILED), "failed"));
    CHECK(td_wifi_retry_ms(0) == 1000 && td_wifi_retry_ms(1) == 2000 && td_wifi_retry_ms(3) == 8000);
    CHECK(td_wifi_retry_ms(4) == 30000 && td_wifi_retry_ms(1000) == 30000);
}

/* ---- Store ---- */

static void test_store(void)
{
    memset(&g_nvs, 0, sizeof g_nvs);
    char ssid[TD_WIFI_SSID_MAX + 1], password[TD_WIFI_PASSWORD_MAX + 1];
    CHECK(td_wifi_store_load(ssid, password) == ESP_ERR_NOT_FOUND);
    CHECK(td_wifi_store_save(SSID, PASSWORD) == ESP_OK);
    CHECK(g_nvs.writes == 1 && g_nvs.len == 3 + strlen(SSID) + strlen(PASSWORD));
    CHECK(td_wifi_store_load(ssid, password) == ESP_OK);
    CHECK(!strcmp(ssid, SSID) && !strcmp(password, PASSWORD));
    CHECK(td_wifi_store_save("", PASSWORD) == ESP_ERR_INVALID_ARG && g_nvs.writes == 1);

    /* A write that fails keeps the old network. */
    g_nvs.set_err = ESP_ERR_NVS_NOT_ENOUGH_SPACE;
    CHECK(td_wifi_store_save(SSID2, PASSWORD2) == ESP_ERR_NVS_NOT_ENOUGH_SPACE);
    g_nvs.set_err = ESP_OK;
    CHECK(td_wifi_store_load(ssid, password) == ESP_OK && !strcmp(ssid, SSID));

    /* A blob of another version or a broken length loads as nothing stored. */
    g_nvs.blob[0] = 2;
    CHECK(td_wifi_store_load(ssid, password) == ESP_ERR_NOT_FOUND && ssid[0] == '\0' && password[0] == '\0');
    g_nvs.blob[0] = 1;
    g_nvs.len--;
    CHECK(td_wifi_store_load(ssid, password) == ESP_ERR_NOT_FOUND);
    g_nvs.len++;
    CHECK(td_wifi_store_load(ssid, password) == ESP_OK);

    CHECK(td_wifi_store_erase() == ESP_OK && !g_nvs.has);
    CHECK(td_wifi_store_erase() == ESP_OK);
    CHECK(td_wifi_store_load(ssid, password) == ESP_ERR_NOT_FOUND);
    CHECK(g_nvs.open_handles == 0);
}

/* ---- The station ---- */

static void test_nothing_stored(void)
{
    fresh(true);
    td_wifi_state_t state = td_wifi_state();
    CHECK(!state.configured && state.status == TD_WIFI_OFF && state.ssid[0] == '\0');
    CHECK(td_wifi_status() == TD_WIFI_OFF);
    CHECK(g_drv.joins == 0);
    CHECK(td_wifi_init(&g_driver) == ESP_ERR_INVALID_STATE);
}

static void test_set_and_join(void)
{
    fresh(true);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    CHECK(g_drv.joins == 1 && !strcmp(g_drv.ssid, SSID) && !strcmp(g_drv.password, PASSWORD));
    CHECK(g_drv.leaves == 0);
    td_wifi_state_t state = td_wifi_state();
    CHECK(state.configured && !strcmp(state.ssid, SSID) && state.status == TD_WIFI_CONNECTING);
    CHECK(td_wifi_status() == TD_WIFI_CONNECTING);

    td_wifi_on_associated();
    CHECK(g_drv.dhcp_ms == TD_WIFI_DHCP_MS);
    CHECK(td_wifi_state().status == TD_WIFI_CONNECTING);
    td_wifi_on_got_ip("192.0.2.10");
    state = td_wifi_state();
    CHECK(state.status == TD_WIFI_CONNECTED && state.reason == TD_WIFI_REASON_NONE);
    CHECK(!strcmp(state.ip, "192.0.2.10") && state.has_rssi && state.rssi == -52);
    CHECK(g_drv.dhcp_ms == 0);

    /* The access point goes silent: lost, and the station tries again. */
    td_wifi_on_disconnected(200, ssid_bytes(SSID), strlen(SSID));
    state = td_wifi_state();
    CHECK(state.status == TD_WIFI_FAILED && state.reason == TD_WIFI_REASON_LOST && state.ip[0] == '\0');
    CHECK(g_drv.retry_ms == 1000);
    td_wifi_on_retry();
    CHECK(g_drv.joins == 2);
}

static void test_not_found_retries_with_backoff(void)
{
    fresh(true);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    const uint32_t expected[] = { 1000, 2000, 4000, 8000, 30000, 30000 };
    for (size_t i = 0; i < sizeof expected / sizeof expected[0]; i++) {
        td_wifi_on_disconnected(201, ssid_bytes(SSID), strlen(SSID));
        td_wifi_state_t state = td_wifi_state();
        CHECK(state.status == TD_WIFI_FAILED && state.reason == TD_WIFI_REASON_NOT_FOUND);
        CHECK(g_drv.retry_ms == expected[i]);
        td_wifi_on_retry();
        CHECK(g_drv.joins == (int)i + 2);
        /* The failure stays shown while the retry runs. */
        CHECK(td_wifi_state().status == TD_WIFI_FAILED);
    }
    /* A new set starts the backoff over. */
    CHECK(td_wifi_set(SSID, PASSWORD2) == ESP_OK);
    CHECK(g_drv.leaves == 1);
    td_wifi_on_disconnected(202, ssid_bytes(SSID), strlen(SSID));
    CHECK(td_wifi_state().reason == TD_WIFI_REASON_WRONG_PASSWORD && g_drv.retry_ms == 1000);
}

static void test_ignored_disconnects(void)
{
    fresh(true);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    /* The station leaving on request decides nothing here. */
    td_wifi_on_disconnected(TD_WIFI_DRIVER_ASSOC_LEAVE, ssid_bytes(SSID), strlen(SSID));
    CHECK(td_wifi_state().status == TD_WIFI_CONNECTING && g_drv.retry_ms == 0);
    /* Nor does the disconnect of the network a set replaced. */
    CHECK(td_wifi_set(SSID2, PASSWORD2) == ESP_OK);
    td_wifi_on_disconnected(201, ssid_bytes(SSID), strlen(SSID));
    CHECK(td_wifi_state().status == TD_WIFI_CONNECTING && g_drv.retry_ms == 0);
    td_wifi_on_disconnected(201, ssid_bytes(SSID2), strlen(SSID2));
    CHECK(td_wifi_state().status == TD_WIFI_FAILED);
    /* A retry while associated waits for the address instead of joining again. */
    td_wifi_on_associated();
    const int joins = g_drv.joins;
    td_wifi_on_retry();
    CHECK(g_drv.joins == joins);
}

static void test_no_address(void)
{
    fresh(true);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    td_wifi_on_associated();
    td_wifi_on_dhcp_timeout();
    td_wifi_state_t state = td_wifi_state();
    CHECK(state.status == TD_WIFI_FAILED && state.reason == TD_WIFI_REASON_NO_IP);
    CHECK(g_drv.leaves == 1 && g_drv.retry_ms == 1000);
    /* A late timeout after the address came does nothing. */
    td_wifi_on_retry();
    td_wifi_on_associated();
    td_wifi_on_got_ip("192.0.2.11");
    td_wifi_on_dhcp_timeout();
    CHECK(td_wifi_state().status == TD_WIFI_CONNECTED && g_drv.leaves == 1);
    /* Losing the address counts as no-ip too. */
    td_wifi_on_lost_ip();
    state = td_wifi_state();
    CHECK(state.status == TD_WIFI_FAILED && state.reason == TD_WIFI_REASON_NO_IP && g_drv.leaves == 2);
}

static void test_driver_refuses(void)
{
    fresh(true);
    g_drv.refuse = true;
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    td_wifi_state_t state = td_wifi_state();
    CHECK(state.status == TD_WIFI_FAILED && state.reason == TD_WIFI_REASON_DRIVER && g_drv.retry_ms == 1000);
    g_drv.refuse = false;
    td_wifi_on_retry();
    CHECK(g_drv.joins == 2);
}

static void test_station_not_started(void)
{
    fresh(false);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    CHECK(g_drv.joins == 0);
    td_wifi_state_t state = td_wifi_state();
    CHECK(state.configured && state.status == TD_WIFI_FAILED && state.reason == TD_WIFI_REASON_DRIVER);
    /* The station starts late and joins then. */
    td_wifi_started(ESP_OK);
    CHECK(g_drv.joins == 1);
    /* A station that never starts says so. */
    reboot();
    CHECK(td_wifi_init(&g_driver) == ESP_OK);
    td_wifi_started(ESP_FAIL);
    CHECK(td_wifi_state().status == TD_WIFI_FAILED && g_drv.joins == 0);
}

static void test_forget(void)
{
    fresh(true);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    td_wifi_on_associated();
    td_wifi_on_got_ip("192.0.2.12");
    CHECK(td_wifi_forget() == ESP_OK);
    td_wifi_state_t state = td_wifi_state();
    CHECK(!state.configured && state.status == TD_WIFI_OFF && state.ssid[0] == '\0' && state.ip[0] == '\0');
    CHECK(g_drv.leaves == 1 && !g_nvs.has && td_wifi_status() == TD_WIFI_OFF);
    CHECK(s_password[0] == '\0');
    /* Events after a forget change nothing. */
    td_wifi_on_disconnected(201, NULL, 0);
    td_wifi_on_retry();
    CHECK(td_wifi_state().status == TD_WIFI_OFF && g_drv.joins == 1);
    /* Forgetting nothing is fine and leaves nothing. */
    CHECK(td_wifi_forget() == ESP_OK && g_drv.leaves == 1);
}

static void test_rejoins_after_reboot(void)
{
    fresh(true);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    reboot();
    CHECK(td_wifi_init(&g_driver) == ESP_OK);
    td_wifi_state_t state = td_wifi_state();
    CHECK(state.configured && !strcmp(state.ssid, SSID) && state.status == TD_WIFI_CONNECTING);
    CHECK(g_drv.joins == 0);
    td_wifi_started(ESP_OK);
    CHECK(g_drv.joins == 1 && !strcmp(g_drv.ssid, SSID) && !strcmp(g_drv.password, PASSWORD));
    /* A forget survives a reboot too. */
    CHECK(td_wifi_forget() == ESP_OK);
    reboot();
    CHECK(td_wifi_init(&g_driver) == ESP_OK);
    td_wifi_started(ESP_OK);
    CHECK(!td_wifi_state().configured && g_drv.joins == 0);
}

static void test_storage_failure_changes_nothing(void)
{
    fresh(true);
    CHECK(td_wifi_set(SSID, PASSWORD) == ESP_OK);
    g_nvs.commit_err = ESP_ERR_NVS_NOT_ENOUGH_SPACE;
    CHECK(td_wifi_set(SSID2, PASSWORD2) == ESP_ERR_NVS_NOT_ENOUGH_SPACE);
    CHECK(!strcmp(td_wifi_state().ssid, SSID) && g_drv.joins == 1 && g_drv.leaves == 0);
    CHECK(td_wifi_forget() == ESP_ERR_NVS_NOT_ENOUGH_SPACE);
    CHECK(td_wifi_state().configured);
    g_nvs.commit_err = ESP_OK;
    CHECK(td_wifi_set("", PASSWORD) == ESP_ERR_INVALID_ARG);
    CHECK(td_wifi_set(SSID, "short") == ESP_ERR_INVALID_ARG);
    CHECK(!strcmp(td_wifi_state().ssid, SSID));
}

/* ---- Messages ---- */

static void test_link_messages(void)
{
    fresh(true);
    cJSON *reply = message("{\"m\":\"wifi.state\"}");
    CHECK(reply_ok(reply) && cJSON_IsFalse(cJSON_GetObjectItemCaseSensitive(reply, "configured")));
    CHECK(!strcmp(reply_str(reply, "state"), "off") && !strcmp(reply_str(reply, "ssid"), ""));
    CHECK(reply_null(reply, "reason") && reply_null(reply, "ip") && reply_null(reply, "rssi"));
    cJSON_Delete(reply);

    reply = message("{\"m\":\"wifi.set\",\"ssid\":\"" SSID "\",\"password\":\"" PASSWORD "\"}");
    CHECK(reply_ok(reply) && !strcmp(reply_str(reply, "ssid"), SSID) && !strcmp(reply_str(reply, "state"), "connecting"));
    check_reply_secret_free(reply);
    cJSON_Delete(reply);
    CHECK(!strcmp(g_drv.password, PASSWORD));

    td_wifi_on_disconnected(201, ssid_bytes(SSID), strlen(SSID));
    reply = message("{\"m\":\"wifi.state\"}");
    CHECK(!strcmp(reply_str(reply, "state"), "failed") && !strcmp(reply_str(reply, "reason"), "not-found"));
    CHECK(reply_null(reply, "ip"));
    check_reply_secret_free(reply);
    cJSON_Delete(reply);

    td_wifi_on_retry();
    td_wifi_on_associated();
    td_wifi_on_got_ip("192.0.2.20");
    reply = message("{\"m\":\"wifi.state\"}");
    CHECK(!strcmp(reply_str(reply, "state"), "connected") && !strcmp(reply_str(reply, "ip"), "192.0.2.20"));
    CHECK(cJSON_GetObjectItemCaseSensitive(reply, "rssi")->valueint == -52);
    check_reply_secret_free(reply);
    cJSON_Delete(reply);

    reply = message("{\"m\":\"wifi.forget\"}");
    CHECK(reply_ok(reply) && !strcmp(reply_str(reply, "state"), "off"));
    cJSON_Delete(reply);
    CHECK(!g_nvs.has);
}

static void test_link_refusals(void)
{
    fresh(true);
    static const char *const bad[] = {
        "{\"m\":\"wifi.set\",\"ssid\":\"" SSID "\"}",
        "{\"m\":\"wifi.set\",\"password\":\"" PASSWORD "\"}",
        "{\"m\":\"wifi.set\",\"ssid\":7,\"password\":\"" PASSWORD "\"}",
        "{\"m\":\"wifi.set\",\"ssid\":\"" SSID "\",\"password\":\"short\"}",
        "{\"m\":\"wifi.set\",\"ssid\":\"\",\"password\":\"" PASSWORD "\"}",
        "{\"m\":\"wifi.set\",\"ssid\":\"" SSID "\",\"password\":\"" PASSWORD "\",\"hidden\":true}",
        "{\"m\":\"wifi.set\",\"ssid\":\"" SSID "\",\"ssid\":\"" SSID2 "\",\"password\":\"" PASSWORD "\"}",
        "{\"m\":\"wifi.state\",\"verbose\":true}",
        "{\"m\":\"wifi.forget\",\"ssid\":\"" SSID "\"}",
    };
    for (size_t i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        cJSON *reply = message(bad[i]);
        CHECK(!reply_ok(reply) && !strcmp(reply_str(reply, "reason"), "args"));
        check_reply_secret_free(reply);
        cJSON_Delete(reply);
    }
    CHECK(g_nvs.writes == 0 && g_drv.joins == 0);

    cJSON *reply = message("{\"m\":\"wifi.scan\"}");
    CHECK(!reply_ok(reply) && !strcmp(reply_str(reply, "reason"), "unsupported"));
    cJSON_Delete(reply);

    g_nvs.commit_err = ESP_ERR_NVS_NOT_ENOUGH_SPACE;
    reply = message("{\"m\":\"wifi.set\",\"ssid\":\"" SSID "\",\"password\":\"" PASSWORD "\"}");
    CHECK(!reply_ok(reply) && !strcmp(reply_str(reply, "reason"), "storage"));
    check_reply_secret_free(reply);
    cJSON_Delete(reply);
    g_nvs.commit_err = ESP_OK;
}

static void test_console_command(void)
{
    fresh(true);
    g_console_len = 0;
    const char *const set[] = { "wifi", "set", SSID, PASSWORD };
    console(4, set);
    CHECK(strstr(g_console, "@td {\"cmd\":\"wifi\",\"ok\":true") == g_console);
    CHECK(strstr(g_console, "\"state\":\"connecting\"") && strstr(g_console, "\"ssid\":\"" SSID "\""));
    CHECK(!strcmp(g_drv.password, PASSWORD));

    const char *const state[] = { "wifi", "state" };
    console(2, state);
    const char *const forget[] = { "wifi", "forget" };
    console(2, forget);
    CHECK(!g_nvs.has);

    const char *const usage[] = { "wifi", "set", SSID, PASSWORD, "extra" };
    console(5, usage);
    CHECK(strstr(g_console, "\"reason\":\"args\""));
    const char *const short_password[] = { "wifi", "set", SSID, "short" };
    console(4, short_password);
    CHECK(strstr(g_console, "Invalid network name or password"));

    CHECK(strstr(g_console, PASSWORD) == NULL);
}

/* After everything above, no log line named a password or a network. */
static void test_logs_hold_no_secret(void)
{
    CHECK(g_log_len > 0);
    CHECK(strstr(g_log, PASSWORD) == NULL && strstr(g_log, PASSWORD2) == NULL);
    CHECK(strstr(g_log, SSID) == NULL && strstr(g_log, SSID2) == NULL);
    CHECK(strstr(g_log, "join failed: not-found, retry in 1000 ms") != NULL);
}

/* ---- The console's echo mask ---- */

static const td_echo_mask_t g_masks[] = { { "wifi set", 3 } };

/* Types `input` into a fresh line editor and returns what it echoed and the line it ran. */
static void type(const char *input, char *echoed, char *ran)
{
    td_line_t state = { .masks = g_masks, .mask_count = 1 };
    char line[256];
    size_t echoed_len = 0;
    ran[0] = '\0';
    for (const char *p = input; *p; p++) {
        char echo[3];
        size_t echo_len = 0;
        td_line_status_t status = td_line_feed(&state, (unsigned char)*p, false, line, sizeof line, echo, &echo_len);
        for (size_t i = 0; i < echo_len; i++) {
            if (echo[i] == '\b') {
                if (echoed_len) echoed_len--;
                i += 2; /* "\b \b" erases one */
            } else if (echo[i] != '\r' && echo[i] != '\n') {
                echoed[echoed_len++] = echo[i];
            }
        }
        if ((*p == '\r' || *p == '\n') && status == TD_LINE_OK) {
            strcpy(ran, line);
        }
    }
    echoed[echoed_len] = '\0';
}

static void test_echo_mask(void)
{
    char echoed[256], ran[256];
    type("wifi set " SSID " " PASSWORD "\r", echoed, ran);
    CHECK(!strcmp(echoed, "wifi set " SSID " ***************"));
    CHECK(!strcmp(ran, "wifi set " SSID " " PASSWORD));

    /* Quotes, escapes and doubled spaces split as the console splits them. */
    type("wifi  set \"my net\" \"pa ss\\\"wd1\"\r", echoed, ran);
    CHECK(!strcmp(echoed, "wifi  set \"my net\" ************"));
    CHECK(!strcmp(ran, "wifi  set \"my net\" \"pa ss\\\"wd1\""));
    type("\"wifi\" set n\\ et secret12\r", echoed, ran);
    CHECK(!strcmp(echoed, "\"wifi\" set n\\ et ********"));

    /* Other lines echo as typed. */
    const char *const plain[] = { "wifi state\r", "wifi forget\r", "wifiset a b\r", "wifi sets a bcdefgh1\r",
                                  "wifi set only-ssid\r", "help wifi set a b\r" };
    for (size_t i = 0; i < sizeof plain / sizeof plain[0]; i++) {
        type(plain[i], echoed, ran);
        CHECK(!strncmp(echoed, plain[i], strlen(plain[i]) - 1) && strlen(echoed) == strlen(plain[i]) - 1);
    }

    /* Backspace out of the secret word shows the line again, and the next secret word hides anew. */
    type("wifi set ab x\b\bc ", echoed, ran);
    CHECK(!strcmp(echoed, "wifi set abc "));
    type("wifi set ab x\b\bc d", echoed, ran);
    CHECK(!strcmp(echoed, "wifi set abc *"));

    /* The mask stays for the next line. */
    td_line_t state = { .masks = g_masks, .mask_count = 1 };
    char line[256], echo[3];
    size_t echo_len;
    const char *two = "wifi state\rwifi set n p";
    char last = 0;
    for (const char *p = two; *p; p++) {
        td_line_feed(&state, (unsigned char)*p, false, line, sizeof line, echo, &echo_len);
        if (echo_len) last = echo[0];
    }
    CHECK(last == '*' && state.mask_count == 1);
}

int main(void)
{
    test_credentials();
    test_reasons_and_backoff();
    test_store();
    test_nothing_stored();
    test_set_and_join();
    test_not_found_retries_with_backoff();
    test_ignored_disconnects();
    test_no_address();
    test_driver_refuses();
    test_station_not_started();
    test_forget();
    test_rejoins_after_reboot();
    test_storage_failure_changes_nothing();
    test_link_messages();
    test_link_refusals();
    test_console_command();
    test_logs_hold_no_secret();
    test_echo_mask();
    reboot();
    if (g_failures) {
        fprintf(stderr, "%d checks failed\n", g_failures);
        return 1;
    }
    printf("wifi: 18 tests passed\n");
    return 0;
}
