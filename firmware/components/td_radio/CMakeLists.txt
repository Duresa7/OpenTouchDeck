# Only the radio build has this component (see firmware/CMakeLists.txt). bt is here so NimBLE compiles into the radio
# build; nothing calls it yet, so the linker leaves it out of the image, and the radio guard checks that.
idf_component_register(SRCS "td_radio.c" "td_wifi.c" "td_wifi_policy.c" "td_wifi_store.c" "td_wifi_msgs.c"
                            "td_wifi_glyph.c"
                       INCLUDE_DIRS "include"
                       REQUIRES console espressif__cjson
                       PRIV_REQUIRES td_console td_link td_store display esp_wifi esp_netif esp_event esp_timer heap
                                     nvs_flash bt espressif__esp_lvgl_adapter lvgl__lvgl)
