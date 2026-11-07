#include "td_radio.h"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "nvs_flash.h"
#include "sdkconfig.h"
#include "td_ble.h"
#include "td_console.h"
#include "td_link.h"
#include "td_wifi.h"
#include "td_wifi_glyph.h"
#if CONFIG_TD_QEMU_TEST
#include "esp_eth.h"
#include "esp_eth_mac_openeth.h"
#endif

static const char *TAG = "radio";

/* td_wifi_policy.c maps these reason codes by number, so it builds on the host without ESP-IDF's headers. */
_Static_assert(WIFI_REASON_AUTH_EXPIRE == 2 && WIFI_REASON_AUTH_LEAVE == 3 &&
                   WIFI_REASON_DISASSOC_DUE_TO_INACTIVITY == 4 && WIFI_REASON_ASSOC_TOOMANY == 5 &&
                   WIFI_REASON_ASSOC_LEAVE == TD_WIFI_DRIVER_ASSOC_LEAVE && WIFI_REASON_MIC_FAILURE == 14 &&
                   WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT == 15 && WIFI_REASON_BEACON_TIMEOUT == 200 &&
                   WIFI_REASON_NO_AP_FOUND == 201 && WIFI_REASON_AUTH_FAIL == 202 && WIFI_REASON_ASSOC_FAIL == 203 &&
                   WIFI_REASON_HANDSHAKE_TIMEOUT == 204 && WIFI_REASON_CONNECTION_FAIL == 205 &&
                   WIFI_REASON_AP_TSF_RESET == 206 && WIFI_REASON_NO_AP_FOUND_W_COMPATIBLE_SECURITY == 210 &&
                   WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD == 211 &&
                   WIFI_REASON_NO_AP_FOUND_IN_RSSI_THRESHOLD == 212,
               "ESP-IDF changed a Wi-Fi reason code that td_wifi_policy.c maps");

#define INTERNAL (MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT)

typedef enum { STATION_OFF, STATION_STARTED, STATION_FAILED } station_t;

static station_t s_station = STATION_OFF;
static esp_err_t s_station_err = ESP_OK;
static esp_timer_handle_t s_steady_timer;
static volatile bool s_steady;
static volatile size_t s_steady_free;
static volatile size_t s_steady_largest;

static void steady_report(void *arg)
{
    (void)arg;
    s_steady_free = heap_caps_get_free_size(INTERNAL);
    s_steady_largest = heap_caps_get_largest_free_block(INTERNAL);
    /* The lowest point since startup, which the Wi-Fi station's start sets at boot. */
    const size_t boot_min = heap_caps_get_minimum_free_size(INTERNAL);
    s_steady = true;
    /* The floors are measured with the station up. Without it the figures prove nothing, so they never pass. */
    if (s_station != STATION_STARTED) {
        ESP_LOGE(TAG, "steady free_internal=%u largest_internal=%u, wifi station not started, floor not measured",
                 (unsigned)s_steady_free, (unsigned)s_steady_largest);
        ESP_LOGE(TAG, "boot min_free_internal=%u, wifi station not started, floor not measured", (unsigned)boot_min);
        return;
    }
    if (s_steady_free >= CONFIG_TD_RADIO_INTERNAL_FLOOR) {
        ESP_LOGI(TAG, "steady free_internal=%u largest_internal=%u floor=%u", (unsigned)s_steady_free,
                 (unsigned)s_steady_largest, (unsigned)CONFIG_TD_RADIO_INTERNAL_FLOOR);
    } else {
        ESP_LOGE(TAG, "steady free_internal=%u largest_internal=%u below floor=%u", (unsigned)s_steady_free,
                 (unsigned)s_steady_largest, (unsigned)CONFIG_TD_RADIO_INTERNAL_FLOOR);
    }
    if (boot_min >= CONFIG_TD_RADIO_BOOT_FLOOR) {
        ESP_LOGI(TAG, "boot min_free_internal=%u floor=%u", (unsigned)boot_min, (unsigned)CONFIG_TD_RADIO_BOOT_FLOOR);
    } else {
        ESP_LOGE(TAG, "boot min_free_internal=%u below floor=%u", (unsigned)boot_min,
                 (unsigned)CONFIG_TD_RADIO_BOOT_FLOOR);
    }
}

/* ---- The Wi-Fi driver under the station logic (td_wifi.c) ---- */

ESP_EVENT_DEFINE_BASE(TD_WIFI_EVENT);
enum { TD_WIFI_EVENT_RETRY, TD_WIFI_EVENT_DHCP };

static esp_timer_handle_t s_retry_timer;
static esp_timer_handle_t s_dhcp_timer;

static void wipe(void *p, size_t n)
{
    volatile unsigned char *b = p;
    while (n--) {
        *b++ = 0;
    }
}

static bool driver_join(const char *ssid, const char *password)
{
    wifi_config_t config = { 0 };
    memcpy(config.sta.ssid, ssid, strnlen(ssid, sizeof config.sta.ssid));
    memcpy(config.sta.password, password, strnlen(password, sizeof config.sta.password));
    /* WPA2 or better when there is a password, so the station never falls back to WEP or WPA. */
    config.sta.threshold.authmode = password[0] ? WIFI_AUTH_WPA2_PSK : WIFI_AUTH_OPEN;
    config.sta.pmf_cfg.capable = true;
    config.sta.sae_pwe_h2e = WPA3_SAE_PWE_BOTH;
    esp_err_t err = esp_wifi_set_config(WIFI_IF_STA, &config);
    wipe(&config, sizeof config);
    if (err == ESP_OK) {
        err = esp_wifi_connect();
    }
    if (err != ESP_OK && err != ESP_ERR_WIFI_CONN) {
        ESP_LOGE(TAG, "wifi join not started: %s", esp_err_to_name(err));
        return false;
    }
    return true;
}

static void driver_leave(void)
{
    (void)esp_wifi_disconnect();
}

static void restart_timer(esp_timer_handle_t timer, uint32_t ms)
{
    if (timer) {
        (void)esp_timer_stop(timer);
        (void)esp_timer_start_once(timer, (uint64_t)ms * 1000);
    }
}

static void driver_retry_in(uint32_t ms)
{
    restart_timer(s_retry_timer, ms);
}

static void driver_dhcp_in(uint32_t ms)
{
    restart_timer(s_dhcp_timer, ms);
}

static void driver_stop_timers(void)
{
    if (s_retry_timer) {
        (void)esp_timer_stop(s_retry_timer);
    }
    if (s_dhcp_timer) {
        (void)esp_timer_stop(s_dhcp_timer);
    }
}

static bool driver_rssi(int *dbm)
{
    wifi_ap_record_t ap;
    if (esp_wifi_sta_get_ap_info(&ap) != ESP_OK) {
        return false;
    }
    *dbm = ap.rssi;
    return true;
}

static const td_wifi_driver_t s_driver = {
    .join = driver_join,
    .leave = driver_leave,
    .retry_in = driver_retry_in,
    .dhcp_in = driver_dhcp_in,
    .stop_timers = driver_stop_timers,
    .rssi = driver_rssi,
};

/* The timers fire on the esp_timer task. Their work runs on the event task beside the driver's own events, so the
 * station logic sees everything in one order. When the event queue is full the timer tries again a second later. */
static void timer_fired(void *arg)
{
    const int32_t id = (int32_t)(intptr_t)arg;
    if (esp_event_post(TD_WIFI_EVENT, id, NULL, 0, 0) != ESP_OK) {
        (void)esp_timer_start_once(id == TD_WIFI_EVENT_RETRY ? s_retry_timer : s_dhcp_timer, 1000000);
    }
}

static void on_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    (void)arg;
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_START) {
        td_wifi_started(ESP_OK);
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_CONNECTED) {
        td_wifi_on_associated();
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        const wifi_event_sta_disconnected_t *event = data;
        td_wifi_on_disconnected(event->reason, event->ssid, event->ssid_len);
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        const ip_event_got_ip_t *event = data;
        char ip[16];
        snprintf(ip, sizeof ip, IPSTR, IP2STR(&event->ip_info.ip));
        td_wifi_on_got_ip(ip);
    } else if (base == IP_EVENT && id == IP_EVENT_STA_LOST_IP) {
        td_wifi_on_lost_ip();
    } else if (base == TD_WIFI_EVENT && id == TD_WIFI_EVENT_RETRY) {
        td_wifi_on_retry();
    } else if (base == TD_WIFI_EVENT && id == TD_WIFI_EVENT_DHCP) {
        td_wifi_on_dhcp_timeout();
    }
}

static esp_err_t start_station(void)
{
    /* The PHY keeps its calibration in NVS. The Wi-Fi driver itself keeps nothing there (CONFIG_ESP_WIFI_NVS_ENABLED
     * is off; td_wifi_store.c keeps the network), and td_device opens the same partition, so a second init is a
     * no-op. */
    /* Before the driver logs anything: its INFO lines name the network. */
    td_wifi_quiet_logs();
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) return err;
    err = esp_netif_init();
    if (err != ESP_OK) return err;
    err = esp_event_loop_create_default();
    if (err != ESP_OK && err != ESP_ERR_INVALID_STATE) return err;
    if (esp_netif_create_default_wifi_sta() == NULL) return ESP_FAIL;
    const esp_timer_create_args_t retry = { .callback = timer_fired, .arg = (void *)(intptr_t)TD_WIFI_EVENT_RETRY,
                                            .name = "td_wifi_retry" };
    const esp_timer_create_args_t dhcp = { .callback = timer_fired, .arg = (void *)(intptr_t)TD_WIFI_EVENT_DHCP,
                                           .name = "td_wifi_dhcp" };
    err = esp_timer_create(&retry, &s_retry_timer);
    if (err != ESP_OK) return err;
    err = esp_timer_create(&dhcp, &s_dhcp_timer);
    if (err != ESP_OK) return err;
    err = esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, on_event, NULL);
    if (err != ESP_OK) return err;
    err = esp_event_handler_register(IP_EVENT, ESP_EVENT_ANY_ID, on_event, NULL);
    if (err != ESP_OK) return err;
    err = esp_event_handler_register(TD_WIFI_EVENT, ESP_EVENT_ANY_ID, on_event, NULL);
    if (err != ESP_OK) return err;
    const wifi_init_config_t config = WIFI_INIT_CONFIG_DEFAULT();
    err = esp_wifi_init(&config);
    if (err != ESP_OK) return err;
    err = esp_wifi_set_storage(WIFI_STORAGE_RAM);
    if (err != ESP_OK) return err;
    err = esp_wifi_set_mode(WIFI_MODE_STA);
    if (err != ESP_OK) return err;
    /* Once the station has started (WIFI_EVENT_STA_START) it joins the stored network, and nothing else: it never
     * scans on its own (esp_wifi_scan_start), and with no network stored it stays unjoined. */
    return esp_wifi_start();
}

/* The `wifi.*` link messages. td_link hands over only USB requests, so a network Carrier never sets, reads or forgets
 * the network. */
static const td_link_extension_t s_wifi_link = {
    .prefix = "wifi.",
    .handle = td_wifi_link_message,
    .caps = td_wifi_link_caps,
};

esp_err_t td_radio_init(void)
{
    /* Each part is set up even when another fails, and the first error is returned. */
    esp_err_t err = td_ble_init();
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "bluetooth keyboard not set up: %s", esp_err_to_name(err));
    }
    esp_err_t wifi_err = td_wifi_init(&s_driver);
    if (wifi_err == ESP_OK) {
        wifi_err = td_link_extend(&s_wifi_link);
    }
    /* The `wifi set` password is masked as it is typed, whatever else failed. */
    const esp_err_t mask_err = td_console_mask_after("wifi set", 3);
    if (wifi_err == ESP_OK) {
        wifi_err = mask_err;
    }
    if (wifi_err != ESP_OK) {
        ESP_LOGE(TAG, "wifi not set up: %s", esp_err_to_name(wifi_err));
    }
    return err != ESP_OK ? err : wifi_err;
}

key_sink_t td_radio_key_sink(void)
{
    return td_ble_key_sink();
}

void td_radio_set_passkey_hook(void (*hook)(bool show, uint32_t passkey))
{
    td_ble_set_passkey_hook(hook);
}

#if CONFIG_TD_QEMU_TEST
/* The QEMU test image only: QEMU's emulated Ethernet (open_eth) with DHCP, in place of the station. Not static, so
 * radio_guard.py can tell from the image that a build carries it. */
esp_err_t td_radio_qemu_eth_start(void);

esp_err_t td_radio_qemu_eth_start(void)
{
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) return err;
    err = esp_netif_init();
    if (err != ESP_OK) return err;
    err = esp_event_loop_create_default();
    if (err != ESP_OK && err != ESP_ERR_INVALID_STATE) return err;
    const esp_netif_inherent_config_t base = ESP_NETIF_INHERENT_DEFAULT_ETH();
    const esp_netif_config_t netif_config = { .base = &base, .stack = ESP_NETIF_NETSTACK_DEFAULT_ETH };
    esp_netif_t *netif = esp_netif_new(&netif_config);
    if (netif == NULL) return ESP_FAIL;
    const eth_mac_config_t mac_config = ETH_MAC_DEFAULT_CONFIG();
    eth_phy_config_t phy_config = ETH_PHY_DEFAULT_CONFIG();
    phy_config.autonego_timeout_ms = 100;
    esp_eth_mac_t *mac = esp_eth_mac_new_openeth(&mac_config);
    esp_eth_phy_t *phy = esp_eth_phy_new_generic(&phy_config);
    if (mac == NULL || phy == NULL) return ESP_FAIL;
    const esp_eth_config_t config = ETH_DEFAULT_CONFIG(mac, phy);
    esp_eth_handle_t eth;
    err = esp_eth_driver_install(&config, &eth);
    if (err != ESP_OK) return err;
    /* A locally administered, synthetic address: QEMU's default eFuse has none. */
    uint8_t address[6] = { 0x02, 0x00, 0x00, 0x00, 0x00, 0x01 };
    err = esp_eth_ioctl(eth, ETH_CMD_S_MAC_ADDR, address);
    if (err != ESP_OK) return err;
    err = esp_netif_attach(netif, esp_eth_new_netif_glue(eth));
    if (err != ESP_OK) return err;
    return esp_eth_start(eth);
}
#endif

esp_err_t td_radio_start(void)
{
#if CONFIG_TD_QEMU_TEST
    const esp_err_t eth_err = td_radio_qemu_eth_start();
    ESP_LOGW(TAG, "QEMU test image: Ethernet %s, no Wi-Fi station; ble dormant", esp_err_to_name(eth_err));
    return eth_err;
#endif
    const esp_err_t err = start_station();
    s_station = err == ESP_OK ? STATION_STARTED : STATION_FAILED;
    s_station_err = err;
    if (err == ESP_OK) {
        ESP_LOGI(TAG, "wifi station started");
    } else {
        ESP_LOGE(TAG, "wifi station failed to start: %s", esp_err_to_name(err));
        td_wifi_started(err);
    }
    /* After the station, so the steady-state figure has both radios when the stored setting turns BLE on. */
    const esp_err_t ble_err = td_ble_boot();
    if (ble_err != ESP_OK) {
        ESP_LOGE(TAG, "ble: %s", esp_err_to_name(ble_err));
    }
    td_wifi_glyph_start();
    const esp_timer_create_args_t args = { .callback = steady_report, .name = "td_radio_steady" };
    esp_err_t timer_err = esp_timer_create(&args, &s_steady_timer);
    if (timer_err == ESP_OK) timer_err = esp_timer_start_once(s_steady_timer, CONFIG_TD_RADIO_STEADY_MS * 1000LL);
    if (timer_err != ESP_OK) {
        ESP_LOGE(TAG, "steady-state report not scheduled: %s", esp_err_to_name(timer_err));
    }
    return err;
}

static const char *station_name(void)
{
    switch (s_station) {
    case STATION_STARTED: return "started";
    case STATION_FAILED: return "failed";
    default: return "off";
    }
}

static int cmd_radio(int argc, char **argv)
{
    char buf[TD_REPLY_MAX];
    if (argc != 2 || strcmp(argv[1], "state") != 0) {
        td_reply_error(buf, sizeof buf, "radio", "usage: radio state");
        td_console_write_line(buf);
        return 0;
    }
    const size_t free_internal = heap_caps_get_free_size(INTERNAL);
    td_reply_t r;
    td_reply_begin(&r, buf, sizeof buf, "radio", true);
    td_reply_str(&r, "op", "state");
    td_reply_str(&r, "build", "radio");
    td_reply_str(&r, "wifi", station_name());
    if (s_station == STATION_FAILED) td_reply_str(&r, "wifi_error", esp_err_to_name(s_station_err));
    td_reply_bool(&r, "joined", td_wifi_status() == TD_WIFI_CONNECTED);
    td_reply_str(&r, "ble", td_ble_state_name());
    td_reply_int(&r, "free_internal", (long long)free_internal);
    td_reply_int(&r, "largest_internal", (long long)heap_caps_get_largest_free_block(INTERNAL));
    td_reply_int(&r, "floor", CONFIG_TD_RADIO_INTERNAL_FLOOR);
    td_reply_bool(&r, "floor_ok", s_station == STATION_STARTED && free_internal >= CONFIG_TD_RADIO_INTERNAL_FLOOR);
    const size_t min_free_internal = heap_caps_get_minimum_free_size(INTERNAL);
    td_reply_int(&r, "min_free_internal", (long long)min_free_internal);
    td_reply_int(&r, "boot_floor", CONFIG_TD_RADIO_BOOT_FLOOR);
    td_reply_bool(&r, "boot_floor_ok",
                  s_station == STATION_STARTED && min_free_internal >= CONFIG_TD_RADIO_BOOT_FLOOR);
    td_reply_bool(&r, "steady", s_steady);
    if (s_steady) {
        td_reply_int(&r, "steady_free_internal", (long long)s_steady_free);
        td_reply_int(&r, "steady_largest_internal", (long long)s_steady_largest);
    }
    td_reply_end(&r);
    td_console_write_line(r.buf);
    return 0;
}

const esp_console_cmd_t *td_radio_console_commands(size_t *count)
{
    /* `radio`, td_ble's `ble` and td_wifi_msgs.c's `wifi`, in one table, so main registers them in one call. */
    static esp_console_cmd_t cmds[3];
    static size_t n;
    if (n == 0) {
        cmds[n++] = (esp_console_cmd_t){ .command = "radio",
                                         .help = "report the radio build's station, BLE and internal heap",
                                         .hint = "state",
                                         .func = cmd_radio };
        size_t ble_count;
        const esp_console_cmd_t *ble = td_ble_console_commands(&ble_count);
        for (size_t i = 0; i < ble_count && n < sizeof cmds / sizeof cmds[0] - 1; i++) {
            cmds[n++] = ble[i];
        }
        size_t wifi_count;
        cmds[n++] = td_wifi_console_commands(&wifi_count)[0];
    }
    *count = n;
    return cmds;
}
