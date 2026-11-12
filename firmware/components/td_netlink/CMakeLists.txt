# net-v2's TCP service. Only the radio build links it (firmware/tools/radio_guard.py).
idf_component_register(SRCS "td_netlink_service.c" "td_netlink_nvs.c" "td_netlink_cmds.c"
                       INCLUDE_DIRS "include"
                       REQUIRES console espressif__cjson
                       PRIV_REQUIRES td_netlink_core td_link td_link_core td_console ui power lwip esp_netif esp_timer
                                     esp_hw_support freertos nvs_flash log heap)
