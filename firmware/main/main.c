#include <stdio.h>
#include <inttypes.h>
#include "sdkconfig.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "cJSON.h"
#include "esp_chip_info.h"
#include "esp_heap_caps.h"
#include "esp_flash.h"
#include "esp_ota_ops.h"
#include "esp_psram.h"
#include "esp_system.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "td_console.h"
#include "td_link.h"
#include "display.h"
#include "touch.h"
#include "ui.h"
#include "key_sink_fanout.h"
#include "key_sink_link.h"
#include "key_sink_log.h"
#include "keys.h"
#include "power.h"
#include "td_recovery.h"
#include "td_store.h"
#include "td_update.h"
#if CONFIG_TD_RADIO_BUILD
#include "td_netlink_service.h"
#include "td_radio.h"
#endif
#if CONFIG_TD_PERF_TOOLS
#include "display_perf.h"
/* The perf tools' boot checkpoints, for `perf mem boot`. */
#define BOOT_MARK(stage) display_perf_boot_mark(stage)
#else
#define BOOT_MARK(stage) ((void)0)
#endif

static const char *TAG = "touchdeck";
static const char *SETTINGS_TAG = "settings";

static key_sink_log_t s_key_log;
static key_sink_fanout_t s_key_fanout;

static void register_commands(const char *owner, const esp_console_cmd_t *cmds, size_t count)
{
    esp_err_t err = td_console_register(cmds, count);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "%s commands not registered: %s", owner, esp_err_to_name(err));
    }
}

/* A layout of up to 32 KB parses into thousands of small cJSON nodes. Below 16 KB malloc prefers internal RAM, which
 * the display and LVGL need, so cJSON takes PSRAM first and internal RAM only when PSRAM is full. */
static void *cjson_malloc(size_t size)
{
    return heap_caps_malloc_prefer(size, 2, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT, MALLOC_CAP_8BIT);
}

/* What the running image must have done before it confirms itself, besides staying up. */
static bool first_frame_shown(void)
{
    return display_frame_count() > 0;
}

static bool layout_loaded(void)
{
    return ui_ready();
}

#ifdef TD_TEST_BOOT_CRASH
/* A build for the rollback check: it dies two seconds after app_main, well before it could confirm itself. */
static void boot_crash_task(void *arg)
{
    (void)arg;
    vTaskDelay(pdMS_TO_TICKS(2000));
    ESP_LOGE(TAG, "TD_TEST_BOOT_CRASH: aborting on purpose");
    abort();
}
#endif

#if CONFIG_TD_RADIO_BUILD
#define HEARTBEAT_US 2000000

/* The radio build's heartbeat runs on the esp_timer task, so app_main can return and give its 6.5 KB stack back to
 * the internal heap that the Wi-Fi station and the panel share. Same log line as the loop below. */
static void heartbeat(void *arg)
{
    static int ticks;
    (void)arg;
#if CONFIG_TD_PERF_TOOLS
    display_perf_mem_sample();
#endif
    ESP_LOGI(TAG, "alive %d, free heap %" PRIu32 " bytes", ticks++, esp_get_free_heap_size());
    if (ticks == 5) {
        BOOT_MARK("alive_10s");
    }
}
#endif

void app_main(void)
{
    BOOT_MARK("app_main");
#ifdef TD_TEST_BOOT_CRASH
    xTaskCreate(boot_crash_task, "td_boot_crash", 3072, NULL, 5, NULL);
#endif
    cJSON_InitHooks(&(cJSON_Hooks){ .malloc_fn = cjson_malloc, .free_fn = heap_caps_free });

    esp_chip_info_t chip_info;
    esp_chip_info(&chip_info);

    uint32_t flash_size = 0;
    esp_flash_get_size(NULL, &flash_size);

    ESP_LOGI(TAG, "TouchDeck firmware starting");
    ESP_LOGI(TAG, "Chip: %s, %d core(s), revision v%d.%d",
             CONFIG_IDF_TARGET, chip_info.cores,
             chip_info.revision / 100, chip_info.revision % 100);
    ESP_LOGI(TAG, "Flash: %" PRIu32 " MB", flash_size / (1024 * 1024));
    // The display frame buffers will need PSRAM, so confirm it came up.
#if CONFIG_SPIRAM
    ESP_LOGI(TAG, "PSRAM: %u KB", (unsigned)(esp_psram_get_size() / 1024));
#else
    /* Only the QEMU test image (firmware/radio/qemu.defaults) has no PSRAM. */
    ESP_LOGI(TAG, "PSRAM: none");
#endif

    const esp_partition_t *running = esp_ota_get_running_partition();
    if (running) {
        ESP_LOGI(TAG, "Running partition %s at 0x%" PRIx32,
                 running->label, running->address);
    } else {
        ESP_LOGW(TAG, "Running partition not found in the partition table");
    }

#if CONFIG_TD_QEMU_TEST
    /* The QEMU test image (firmware/radio/qemu.defaults): QEMU has no panel, touch controller or USB-Serial/JTAG, so
     * it runs only the link and net-v2 over QEMU's Ethernet, for CI's network check. radio_guard.py keeps it out of
     * every real build. */
    size_t qemu_net_count;
    td_netlink_console_commands(&qemu_net_count);
    ESP_ERROR_CHECK(td_link_start());
    ESP_ERROR_CHECK(td_radio_start());
    ESP_ERROR_CHECK(td_netlink_start());
    return;
#endif

    /* td_console_start seals the command list, so every component hands over its commands first. */
    size_t display_cmd_count;
    const esp_console_cmd_t *display_cmds = display_console_commands(&display_cmd_count);
    register_commands("Display", display_cmds, display_cmd_count);
    size_t ui_cmd_count;
    const esp_console_cmd_t *ui_cmds = ui_console_commands(&ui_cmd_count);
    register_commands("UI", ui_cmds, ui_cmd_count);
    size_t touch_cmd_count;
    const esp_console_cmd_t *touch_cmds = touch_console_commands(&touch_cmd_count);
    register_commands("Touch", touch_cmds, touch_cmd_count);
    size_t keys_cmd_count;
    const esp_console_cmd_t *keys_cmds = keys_console_commands(&s_key_log, &keys_cmd_count);
    register_commands("Keys", keys_cmds, keys_cmd_count);
    size_t power_cmd_count;
    const esp_console_cmd_t *power_cmds = power_console_commands(&power_cmd_count);
    register_commands("Power", power_cmds, power_cmd_count);
    size_t link_cmd_count;
    const esp_console_cmd_t *link_cmds = td_link_console_commands(&link_cmd_count);
    register_commands("Link", link_cmds, link_cmd_count);
    size_t store_cmd_count;
    const esp_console_cmd_t *store_cmds = td_store_console_commands(&store_cmd_count);
    register_commands("Store", store_cmds, store_cmd_count);
    size_t recovery_cmd_count;
    const esp_console_cmd_t *recovery_cmds = td_recovery_console_commands(&recovery_cmd_count);
    register_commands("Recovery", recovery_cmds, recovery_cmd_count);
    size_t update_cmd_count;
    const esp_console_cmd_t *update_cmds = td_update_console_commands(&update_cmd_count);
    register_commands("Update", update_cmds, update_cmd_count);
#if CONFIG_TD_RADIO_BUILD
    size_t radio_cmd_count;
    const esp_console_cmd_t *radio_cmds = td_radio_console_commands(&radio_cmd_count);
    register_commands("Radio", radio_cmds, radio_cmd_count);
    size_t net_cmd_count;
    const esp_console_cmd_t *net_cmds = td_netlink_console_commands(&net_cmd_count);
    register_commands("Net", net_cmds, net_cmd_count);
#endif
    BOOT_MARK("commands");

    /* The volume mounts before the console starts, and the settings load before the display does, which sizes its
     * frame buffers for the stored orientation. A change to it applies on the next boot. */
    td_settings_t settings;
    td_settings_defaults(&settings);
    esp_err_t store_err = td_store_mount(NULL);
    BOOT_MARK("store");
    if (store_err == ESP_OK) {
        /* A boot that finds the recovery marker runs the default settings from RAM and leaves settings.json alone. */
        if (!td_recovery_boot()) {
            const esp_err_t settings_err = td_store_settings_load(&settings);
            if (settings_err == ESP_ERR_INVALID_RESPONSE) {
                ESP_LOGW(SETTINGS_TAG, "settings.json is not usable, using the defaults for what it does not give");
            }
        }
    } else {
        ESP_LOGE(TAG, "Store failed to mount, the settings are the defaults: %s", esp_err_to_name(store_err));
    }
    ESP_LOGI(SETTINGS_TAG, "idle_timeout_s=%u orientation=%s", (unsigned)settings.idle_timeout_s,
             td_settings_orientation_name(settings.orientation));
    BOOT_MARK("settings");

    /* The Companion engine reports to the log and link. Bluetooth Tiles use their explicit destination. */
    key_sink_log_start(&s_key_log);
#if CONFIG_TD_RADIO_BUILD
    ui_set_bluetooth_sink(td_radio_key_sink());
#endif
    s_key_fanout = (key_sink_fanout_t){ .sinks = { key_sink_log_sink(&s_key_log), key_sink_link_sink() } };
    esp_err_t keys_err = keys_start(key_sink_fanout_sink(&s_key_fanout));
    if (keys_err != ESP_OK) {
        ESP_LOGE(TAG, "Keys failed to start: %s", esp_err_to_name(keys_err));
    }
    BOOT_MARK("keys");

#if CONFIG_TD_RADIO_BUILD
    /* Before the link and the console start, so a v2 hello carries the Bluetooth and Wi-Fi capabilities, the first
     * `wifi.*` message finds its handler and the first typed `wifi set` password is masked. It starts no radio and
     * logs what failed. A host pairs by typing the passkey the UI shows. */
    td_radio_set_passkey_hook(ui_passkey_set);
    (void)td_radio_init();
#endif
    esp_err_t link_err = td_link_start();
    if (link_err != ESP_OK) {
        ESP_LOGE(TAG, "Link failed to start: %s", esp_err_to_name(link_err));
    }
    BOOT_MARK("link");

    esp_err_t console_err = td_console_start();
    if (console_err != ESP_OK) {
        ESP_LOGE(TAG, "Console failed to start: %s", esp_err_to_name(console_err));
    }
    BOOT_MARK("console");

    /* A new image waits here for its first frame, a layout and ten seconds of uptime before it confirms itself. */
    esp_err_t update_err = td_update_start(first_frame_shown, layout_loaded);
    if (update_err != ESP_OK) {
        ESP_LOGE(TAG, "Update watch failed to start: %s", esp_err_to_name(update_err));
    }
    BOOT_MARK("update");

    ui_set_press_hook(td_link_press);
    ui_set_part_hook(td_link_press_part);
    ui_set_input_hooks(td_link_input, td_link_input_cancel);
    ui_layout_boot(td_settings_orientation_name(settings.orientation));
    BOOT_MARK("layout");
    esp_err_t display_err = display_start(settings.orientation == TD_ORIENTATION_PORTRAIT, ui_build);
    if (display_err != ESP_OK) {
        ESP_LOGE(TAG, "Display failed to start: %s", esp_err_to_name(display_err));
    }
    BOOT_MARK("display");

    /* The gate needs the CH422G that display_start brought up, and it must exist before touch reads start. */
    esp_err_t power_err = power_start(esp_timer_get_time());
    if (power_err != ESP_OK) {
        ESP_LOGE(TAG, "Power failed to start, the screen stays on: %s", esp_err_to_name(power_err));
    } else {
        power_set_idle_timeout_s(settings.idle_timeout_s, esp_timer_get_time());
        td_link_idle_boot();
    }
    BOOT_MARK("power");

    esp_err_t touch_err = touch_start();
    if (touch_err != ESP_OK) {
        ESP_LOGE(TAG, "Touch failed to start: %s", esp_err_to_name(touch_err));
    }
    BOOT_MARK("touch");

#if CONFIG_TD_RADIO_BUILD
    /* Last, so the panel and the render tasks have their internal RAM before the station takes its share. */
    esp_err_t radio_err = td_radio_start();
    if (radio_err != ESP_OK) {
        ESP_LOGE(TAG, "Radio failed to start: %s", esp_err_to_name(radio_err));
    }
    BOOT_MARK("radio");
    /* net-v2 over TCP, after the station so lwIP is up. */
    esp_err_t net_err = td_netlink_start();
    if (net_err != ESP_OK) {
        ESP_LOGE(TAG, "Network link failed to start: %s", esp_err_to_name(net_err));
    }
    BOOT_MARK("net");
#endif

#if CONFIG_TD_RADIO_BUILD
    static const esp_timer_create_args_t heartbeat_args = { .callback = heartbeat, .name = "td_heartbeat" };
    esp_timer_handle_t heartbeat_timer;
    ESP_ERROR_CHECK(esp_timer_create(&heartbeat_args, &heartbeat_timer));
    heartbeat(NULL);
    ESP_ERROR_CHECK(esp_timer_start_periodic(heartbeat_timer, HEARTBEAT_US));
#else
    int ticks = 0;
    while (true) {
#if CONFIG_TD_PERF_TOOLS
        display_perf_mem_sample();
#endif
        ESP_LOGI(TAG, "alive %d, free heap %" PRIu32 " bytes",
                 ticks++, esp_get_free_heap_size());
        vTaskDelay(pdMS_TO_TICKS(2000));
        if (ticks == 5) {
            BOOT_MARK("alive_10s");
        }
    }
#endif
}
