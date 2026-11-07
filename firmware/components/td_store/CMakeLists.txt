idf_component_register(SRCS "td_gif.c" "td_asset.c" "td_device.c" "td_device_name.c" "td_store.c" "td_store_cmds.c" "td_settings.c" "td_recovery.c" "td_recovery_cmds.c"
                       INCLUDE_DIRS "include"
                       REQUIRES console td_idle
                       PRIV_REQUIRES td_update nvs_flash espressif__cjson joltwallet__littlefs td_console power esp_rom esp_system esp_timer freertos)
