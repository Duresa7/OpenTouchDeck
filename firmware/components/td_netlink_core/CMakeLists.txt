# net-v2's protocol core, with no socket or stack: only the radio build links it (firmware/tools/radio_guard.py).
idf_component_register(SRCS "td_netlink.c" "td_netlink_crypto.c" "td_netlink_srp.c" "td_netlink_record.c"
                            "td_netlink_ble.c" "td_netlink_manage.c"
                       INCLUDE_DIRS "include"
                       REQUIRES espressif__cjson
                       PRIV_REQUIRES td_link_core mbedtls)
