# Only the radio build has this component (see firmware/CMakeLists.txt). bt is here so NimBLE compiles into the radio
# build; nothing calls it yet, so the linker leaves it out of the image, and the radio guard checks that.
idf_component_register(SRCS "td_radio.c"
                       INCLUDE_DIRS "include"
                       REQUIRES console
                       PRIV_REQUIRES td_console esp_wifi esp_netif esp_event esp_timer heap nvs_flash bt esp_eth)
