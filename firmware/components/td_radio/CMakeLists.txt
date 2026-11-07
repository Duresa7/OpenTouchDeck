# Only the radio build has this component (see firmware/CMakeLists.txt). td_ble brings NimBLE, which starts only when
# the stored Bluetooth setting is on; the radio guard checks the rest of NimBLE's start-up stays out of the image.
idf_component_register(SRCS "td_radio.c"
                       INCLUDE_DIRS "include"
                       REQUIRES console keys
                       PRIV_REQUIRES td_console td_ble esp_wifi esp_netif esp_event esp_timer heap nvs_flash esp_eth)
