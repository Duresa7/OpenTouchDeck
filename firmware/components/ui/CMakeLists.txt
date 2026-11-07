idf_component_register(SRCS "ui_theme.c" "ui_deck.c" "ui_cmds.c" "ui_tile_keys.c" "ui_icon.c" "ui_layout_cmds.c"
                            "ui_widget.c" "ui_dashboard.c" "ui_live_tiles.c" "ui_idle.c" "ui_gif.c" "ui_pairing.c"
                            "fonts/ui_font_medium_22.c" "fonts/ui_font_medium_26.c" "fonts/ui_font_medium_44.c"
                            "fonts/ui_font_semibold_32.c" "fonts/ui_font_semibold_40.c" "fonts/ui_font_nameplate_26.c"
                       INCLUDE_DIRS "include"
                       REQUIRES console td_layout td_ui_core
                       PRIV_REQUIRES board display keys power touch td_console td_store td_idle esp_timer heap
                                     espressif__esp_lvgl_adapter lvgl__lvgl)
