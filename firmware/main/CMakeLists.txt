idf_component_register(SRCS "main.c"
                       PRIV_REQUIRES spi_flash esp_psram esp_timer app_update td_console td_link td_update display touch ui keys power td_store espressif__cjson heap
                       INCLUDE_DIRS "")

# `idf.py build-crashing` builds with -DTD_TEST_BOOT_CRASH=1 into build-crash/, an image that aborts two seconds after
# app_main so the rollback can be checked on the board.
if(TD_TEST_BOOT_CRASH)
    target_compile_definitions(${COMPONENT_LIB} PRIVATE TD_TEST_BOOT_CRASH=1)
endif()

# Only the radio build (-DTD_RADIO_BUILD=1) has td_radio, whose Kconfig sets CONFIG_TD_RADIO_BUILD. A requirement
# cannot depend on Kconfig, so main links it here.
if(CONFIG_TD_RADIO_BUILD)
    target_link_libraries(${COMPONENT_LIB} PRIVATE idf::td_radio)
endif()
