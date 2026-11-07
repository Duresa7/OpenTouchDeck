#include "td_radio.h"
#include <stdbool.h>
#include <string.h>
#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "nvs_flash.h"
#include "sdkconfig.h"
#include "td_console.h"
#if CONFIG_TD_QEMU_TEST
#include "esp_eth.h"
#include "esp_eth_mac_openeth.h"
#endif

static const char *TAG = "radio";

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
    s_steady = true;
    /* The floor is measured with the station up. Without it the figure proves nothing, so it never passes. */
    if (s_station != STATION_STARTED) {
        ESP_LOGE(TAG, "steady free_internal=%u largest_internal=%u, wifi station not started, floor not measured",
                 (unsigned)s_steady_free, (unsigned)s_steady_largest);
    } else if (s_steady_free >= CONFIG_TD_RADIO_INTERNAL_FLOOR) {
        ESP_LOGI(TAG, "steady free_internal=%u largest_internal=%u floor=%u", (unsigned)s_steady_free,
                 (unsigned)s_steady_largest, (unsigned)CONFIG_TD_RADIO_INTERNAL_FLOOR);
    } else {
        ESP_LOGE(TAG, "steady free_internal=%u largest_internal=%u below floor=%u", (unsigned)s_steady_free,
                 (unsigned)s_steady_largest, (unsigned)CONFIG_TD_RADIO_INTERNAL_FLOOR);
    }
}

static esp_err_t start_station(void)
{
    /* The PHY keeps its calibration in NVS. The Wi-Fi driver itself keeps nothing there (CONFIG_ESP_WIFI_NVS_ENABLED
     * is off), and td_device opens the same partition, so a second init is a no-op. */
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) return err;
    err = esp_netif_init();
    if (err != ESP_OK) return err;
    err = esp_event_loop_create_default();
    if (err != ESP_OK && err != ESP_ERR_INVALID_STATE) return err;
    if (esp_netif_create_default_wifi_sta() == NULL) return ESP_FAIL;
    const wifi_init_config_t config = WIFI_INIT_CONFIG_DEFAULT();
    err = esp_wifi_init(&config);
    if (err != ESP_OK) return err;
    err = esp_wifi_set_storage(WIFI_STORAGE_RAM);
    if (err != ESP_OK) return err;
    err = esp_wifi_set_mode(WIFI_MODE_STA);
    if (err != ESP_OK) return err;
    /* No network is set and nothing calls esp_wifi_connect or esp_wifi_scan_start, so the station neither scans nor
     * joins. It is up so its internal RAM shows in the steady-state figure. */
    return esp_wifi_start();
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
        ESP_LOGI(TAG, "wifi station started, not joined; ble dormant");
    } else {
        ESP_LOGE(TAG, "wifi station failed to start: %s; ble dormant", esp_err_to_name(err));
    }
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
    td_reply_bool(&r, "joined", false);
    td_reply_str(&r, "ble", "dormant");
    td_reply_int(&r, "free_internal", (long long)free_internal);
    td_reply_int(&r, "largest_internal", (long long)heap_caps_get_largest_free_block(INTERNAL));
    td_reply_int(&r, "floor", CONFIG_TD_RADIO_INTERNAL_FLOOR);
    td_reply_bool(&r, "floor_ok", s_station == STATION_STARTED && free_internal >= CONFIG_TD_RADIO_INTERNAL_FLOOR);
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
    static const esp_console_cmd_t cmds[] = {
        { .command = "radio", .help = "report the radio build's station, BLE and internal heap",
          .hint = "state", .func = cmd_radio },
    };
    *count = sizeof cmds / sizeof cmds[0];
    return cmds;
}
