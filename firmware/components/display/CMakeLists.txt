set(srcs "display.c" "display_cmds.c" "panel_rgb.c" "display_rotate.c")
if(CONFIG_TD_PERF_TOOLS)
    list(APPEND srcs "display_perf.c" "display_perf_cmds.c" "display_perf_fb.c" "display_perf_mem.c")
endif()
if(CONFIG_TD_PAY_COPY_DEBT)
    list(APPEND srcs "display_copy_debt.c")
endif()
# LVGL's PSRAM-first allocator, which only the radio build selects (radio/radio.defaults).
if(CONFIG_LV_USE_CUSTOM_MALLOC)
    list(APPEND srcs "display_lv_mem.c")
endif()

# esp_driver_tsens and esp_mm serve only the perf tools, but requirements are read before the configuration is, so
# they cannot depend on it. Without the perf tools nothing links from them.
idf_component_register(SRCS ${srcs}
                       INCLUDE_DIRS "include"
                       REQUIRES console
                       PRIV_REQUIRES board td_shot td_console td_rotate esp_lcd esp_timer
                                     espressif__esp_lvgl_adapter lvgl__lvgl esp_driver_tsens esp_mm)

# display_rotate.c replaces esp_lvgl_adapter's software rotation copy by wrapping the symbol at link time, so the
# managed component stays as the registry ships it. The wrapper relies on that function's arguments and mapping in
# the pinned 0.7.3, so stop the build if another version arrives until someone rechecks it.
set(td_adapter_version "0.7.3")
idf_component_get_property(td_adapter_dir espressif__esp_lvgl_adapter COMPONENT_DIR)
file(STRINGS "${td_adapter_dir}/idf_component.yml" td_adapter_version_line REGEX "^version:")
if(NOT td_adapter_version_line MATCHES "^version: *['\"]?${td_adapter_version}['\"]?$")
    message(FATAL_ERROR "display_rotate.c wraps esp_lvgl_adapter ${td_adapter_version}'s display_rotate_copy_region, "
                        "but the build has '${td_adapter_version_line}'. Recheck the wrapper against it first.")
endif()
target_link_libraries(${COMPONENT_LIB} INTERFACE "-Wl,--wrap=display_rotate_copy_region"
                                                 "-u __wrap_display_rotate_copy_region")

# Test builds only: the perf tools count frame hand-overs and panel refreshes by wrapping the esp_lcd calls the adapter
# makes, and leave the adapter's own callbacks in place (see display_perf.c).
if(CONFIG_TD_PERF_TOOLS)
    target_link_libraries(${COMPONENT_LIB} INTERFACE "-Wl,--wrap=esp_lcd_rgb_panel_register_event_callbacks"
                                                     "-u __wrap_esp_lcd_rgb_panel_register_event_callbacks"
                                                     "-Wl,--wrap=esp_lcd_panel_draw_bitmap"
                                                     "-u __wrap_esp_lcd_panel_draw_bitmap")
endif()

# LVGL calls its allocator, but the linker reads this component's library before LVGL's, so ask for the object up
# front or nothing pulls it in.
if(CONFIG_LV_USE_CUSTOM_MALLOC)
    target_link_libraries(${COMPONENT_LIB} INTERFACE "-u lv_mem_init")
endif()
