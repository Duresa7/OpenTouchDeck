# The radio build's Bluetooth keyboard. Only td_radio requires it, and only the radio build has td_radio, so the normal
# build never compiles NimBLE or this component (firmware/tools/radio_guard.py checks both). td_ble_hid.c, td_ble_core.c
# and td_ble_msgs.c have no ESP-IDF in them; the host tests build them with a fake port (firmware/tools/host/).
idf_component_register(SRCS "td_ble_hid.c" "td_ble_core.c" "td_ble_msgs.c" "td_ble_cmds.c" "td_ble_nimble.c"
                       INCLUDE_DIRS "include"
                       PRIV_INCLUDE_DIRS "."
                       REQUIRES console keys
                       PRIV_REQUIRES bt nvs_flash esp_timer heap freertos td_console td_link td_store espressif__cjson)
