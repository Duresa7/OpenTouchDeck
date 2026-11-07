#include "td_wifi.h"
#include <stdio.h>
#include <string.h>
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

static const char *TAG = "td_wifi";

/*
 * The station logic. One lock guards everything here and is held across the driver calls, so a set, a forget and the
 * driver's events apply one at a time and in order. The NVS write of a set or forget happens under it too: it is rare,
 * and the indicator reads the status without the lock.
 *
 * Our log lines name no network and never the password: the SSID is the person's, and the console log reaches the
 * Companion. ESP-IDF's own Wi-Fi and supplicant lines do name it ("connected with <SSID>, bssid=..."), and the netif
 * handler logs the address it got, so td_wifi_quiet_logs() turns those tags down before the station starts. Our tag is
 * td_wifi, so turning down `wifi` keeps our lines.
 */
static SemaphoreHandle_t s_lock;
static const td_wifi_driver_t *s_driver;
static td_wifi_state_t s_state;
static char s_password[TD_WIFI_PASSWORD_MAX + 1];
static volatile td_wifi_status_t s_status;
/* The station started; until then a stored network waits. */
static bool s_started;
/* Joined the access point and waiting for an address. */
static bool s_associated;
/* Failed joins since the last success or set, for the retry backoff. */
static unsigned s_attempt;

static void wipe(void *p, size_t n)
{
    volatile unsigned char *b = p;
    while (n--) {
        *b++ = 0;
    }
}

void td_wifi_quiet_logs(void)
{
    static const char *const tags[] = { "wifi", "wpa", "esp_netif_handlers" };
    for (size_t i = 0; i < sizeof tags / sizeof tags[0]; i++) {
        esp_log_level_set(tags[i], ESP_LOG_ERROR);
    }
}

static void lock(void)
{
    xSemaphoreTake(s_lock, portMAX_DELAY);
}

static void unlock(void)
{
    s_status = s_state.status;
    xSemaphoreGive(s_lock);
}

static void clear_link(void)
{
    s_associated = false;
    s_state.ip[0] = '\0';
    s_state.rssi = 0;
    s_state.has_rssi = false;
}

/* Starts one join of the stored network. Call with the lock held. */
static void join(void)
{
    if (!s_driver->join(s_state.ssid, s_password)) {
        s_state.status = TD_WIFI_FAILED;
        s_state.reason = TD_WIFI_REASON_DRIVER;
        s_driver->retry_in(td_wifi_retry_ms(s_attempt));
        if (s_attempt < 100) {
            s_attempt++;
        }
    }
}

/* The join failed or the connection dropped: say why and try again later. Call with the lock held. */
static void fail(td_wifi_reason_t reason)
{
    clear_link();
    s_state.status = TD_WIFI_FAILED;
    s_state.reason = reason;
    s_driver->stop_timers();
    s_driver->retry_in(td_wifi_retry_ms(s_attempt));
    if (s_attempt < 100) {
        s_attempt++;
    }
    ESP_LOGW(TAG, "join failed: %s, retry in %u ms", td_wifi_reason_name(reason),
             (unsigned)td_wifi_retry_ms(s_attempt - 1));
}

esp_err_t td_wifi_init(const td_wifi_driver_t *driver)
{
    if (s_lock) {
        return ESP_ERR_INVALID_STATE;
    }
    if (!driver || !driver->join || !driver->leave || !driver->retry_in || !driver->dhcp_in || !driver->stop_timers) {
        return ESP_ERR_INVALID_ARG;
    }
    s_lock = xSemaphoreCreateMutex();
    if (!s_lock) {
        return ESP_ERR_NO_MEM;
    }
    s_driver = driver;
    lock();
    const esp_err_t err = td_wifi_store_load(s_state.ssid, s_password);
    if (err == ESP_OK) {
        s_state.configured = true;
        s_state.status = TD_WIFI_CONNECTING;
        ESP_LOGI(TAG, "a network is stored");
    } else if (err != ESP_ERR_NOT_FOUND) {
        ESP_LOGE(TAG, "the stored network could not be read: %s", esp_err_to_name(err));
    }
    unlock();
    return err == ESP_ERR_NOT_FOUND ? ESP_OK : err;
}

void td_wifi_started(esp_err_t err)
{
    if (!s_lock) {
        return;
    }
    lock();
    s_started = err == ESP_OK;
    if (!s_started && s_state.configured) {
        s_state.status = TD_WIFI_FAILED;
        s_state.reason = TD_WIFI_REASON_DRIVER;
    } else if (s_started && s_state.configured) {
        join();
    }
    unlock();
}

esp_err_t td_wifi_set(const char *ssid, const char *password)
{
    if (!td_wifi_credentials_valid(ssid, password)) {
        return ESP_ERR_INVALID_ARG;
    }
    if (!s_lock) {
        return ESP_ERR_INVALID_STATE;
    }
    lock();
    const esp_err_t err = td_wifi_store_save(ssid, password);
    if (err == ESP_OK) {
        s_driver->stop_timers();
        if (s_started && s_state.configured) {
            s_driver->leave();
        }
        snprintf(s_state.ssid, sizeof s_state.ssid, "%s", ssid);
        wipe(s_password, sizeof s_password);
        snprintf(s_password, sizeof s_password, "%s", password);
        s_state.configured = true;
        s_state.status = s_started ? TD_WIFI_CONNECTING : TD_WIFI_FAILED;
        s_state.reason = s_started ? TD_WIFI_REASON_NONE : TD_WIFI_REASON_DRIVER;
        clear_link();
        s_attempt = 0;
        ESP_LOGI(TAG, "network set%s", s_started ? ", joining" : ", the station is not running");
        if (s_started) {
            join();
        }
    } else {
        ESP_LOGE(TAG, "the network was not stored: %s", esp_err_to_name(err));
    }
    unlock();
    return err;
}

esp_err_t td_wifi_forget(void)
{
    if (!s_lock) {
        return ESP_ERR_INVALID_STATE;
    }
    lock();
    const esp_err_t err = td_wifi_store_erase();
    if (err == ESP_OK) {
        s_driver->stop_timers();
        if (s_started && s_state.configured) {
            s_driver->leave();
        }
        wipe(s_password, sizeof s_password);
        memset(&s_state, 0, sizeof s_state);
        s_state.status = TD_WIFI_OFF;
        clear_link();
        s_attempt = 0;
        ESP_LOGI(TAG, "network forgotten");
    } else {
        ESP_LOGE(TAG, "the network was not erased: %s", esp_err_to_name(err));
    }
    unlock();
    return err;
}

td_wifi_state_t td_wifi_state(void)
{
    if (!s_lock) {
        return (td_wifi_state_t){ .status = TD_WIFI_OFF };
    }
    lock();
    td_wifi_state_t state = s_state;
    if (state.status == TD_WIFI_CONNECTED && s_driver->rssi && s_driver->rssi(&state.rssi)) {
        state.has_rssi = true;
    }
    unlock();
    return state;
}

td_wifi_status_t td_wifi_status(void)
{
    return s_status;
}

void td_wifi_on_associated(void)
{
    if (!s_lock) {
        return;
    }
    lock();
    if (s_state.configured && s_state.status != TD_WIFI_CONNECTED) {
        s_associated = true;
        s_driver->stop_timers();
        s_driver->dhcp_in(TD_WIFI_DHCP_MS);
    }
    unlock();
}

void td_wifi_on_disconnected(uint16_t driver_reason, const uint8_t *ssid, size_t ssid_len)
{
    if (!s_lock) {
        return;
    }
    lock();
    /* The station leaving on request is a set, a forget or a failure here, which has already decided what comes next.
     * A disconnect that names another network is the one a set replaced. */
    const bool stale = ssid_len != 0 && (ssid_len != strlen(s_state.ssid) || memcmp(ssid, s_state.ssid, ssid_len));
    if (s_state.configured && driver_reason != TD_WIFI_DRIVER_ASSOC_LEAVE && !stale) {
        const bool was_up = s_state.status == TD_WIFI_CONNECTED;
        const td_wifi_reason_t reason = td_wifi_reason_from_driver(driver_reason);
        fail(was_up && reason == TD_WIFI_REASON_OTHER ? TD_WIFI_REASON_LOST : reason);
    }
    unlock();
}

void td_wifi_on_got_ip(const char *ip)
{
    if (!s_lock) {
        return;
    }
    lock();
    if (s_state.configured) {
        s_driver->stop_timers();
        s_associated = false;
        snprintf(s_state.ip, sizeof s_state.ip, "%s", ip);
        s_state.status = TD_WIFI_CONNECTED;
        s_state.reason = TD_WIFI_REASON_NONE;
        s_attempt = 0;
        ESP_LOGI(TAG, "joined");
    }
    unlock();
}

void td_wifi_on_lost_ip(void)
{
    if (!s_lock) {
        return;
    }
    lock();
    if (s_state.configured && s_state.status == TD_WIFI_CONNECTED) {
        s_driver->leave();
        fail(TD_WIFI_REASON_NO_IP);
    }
    unlock();
}

void td_wifi_on_retry(void)
{
    if (!s_lock) {
        return;
    }
    lock();
    if (s_started && s_state.configured && s_state.status != TD_WIFI_CONNECTED && !s_associated) {
        join();
    }
    unlock();
}

void td_wifi_on_dhcp_timeout(void)
{
    if (!s_lock) {
        return;
    }
    lock();
    if (s_state.configured && s_associated && s_state.status != TD_WIFI_CONNECTED) {
        s_driver->leave();
        fail(TD_WIFI_REASON_NO_IP);
    }
    unlock();
}
