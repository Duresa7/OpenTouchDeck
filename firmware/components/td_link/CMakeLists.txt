idf_component_register(SRCS "td_link.c" "td_link_msgs.c" "td_link_cmds.c" "td_link_files.c" "td_link_transfer.c" "td_link_live.c" "td_link_idle.c" "key_sink_link.c"
                       INCLUDE_DIRS "include"
                       REQUIRES td_console td_link_core espressif__cjson
                       PRIV_REQUIRES esp_hw_support td_ui_core keys td_layout td_store td_update td_idle ui power esp_timer esp_app_format app_update heap)
