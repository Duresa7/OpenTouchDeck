# ESP-IDF project root. Firmware sources live in main/.
cmake_minimum_required(VERSION 3.22)

# The radio build: `idf.py -B <dir> -DTD_RADIO_BUILD=1 build`. It adds the td_radio component, which brings Wi-Fi,
# lwIP and NimBLE, and layers radio/radio.defaults over the defaults. Its sdkconfig lives in its build directory, so
# the committed sdkconfig stays the normal, radio-free one. CMake saves the option in the build directory's cache, so
# every later build there is a radio build too: use a build directory of its own. The default build/ is refused, because
# verify-touchdeck and a plain `idf.py flash` take the normal Firmware from it.
if(TD_RADIO_BUILD)
    if(CMAKE_BINARY_DIR STREQUAL "${CMAKE_SOURCE_DIR}/build")
        message(FATAL_ERROR "The radio build needs a build directory of its own (idf.py -B <dir> -DTD_RADIO_BUILD=1). "
                            "firmware/build is for the normal Firmware.")
    endif()
    if(NOT DEFINED SDKCONFIG)
        set(SDKCONFIG "${CMAKE_BINARY_DIR}/sdkconfig")
    endif()
    if(NOT DEFINED SDKCONFIG_DEFAULTS)
        if(DEFINED ENV{SDKCONFIG_DEFAULTS})
            set(SDKCONFIG_DEFAULTS "$ENV{SDKCONFIG_DEFAULTS}")
        else()
            set(SDKCONFIG_DEFAULTS "sdkconfig.defaults")
        endif()
    endif()
    list(APPEND SDKCONFIG_DEFAULTS "radio/radio.defaults")
    # CI's QEMU image of the radio build (-DTD_RADIO_BUILD=1 -DTD_QEMU_TEST=1): Ethernet in place of Wi-Fi and a
    # pairing window at boot. Never flashed to a Device; radio_guard.py fails any build that has it.
    if(TD_QEMU_TEST)
        list(APPEND SDKCONFIG_DEFAULTS "radio/qemu.defaults")
    endif()
    # The same components as the normal build's minimal build, main and what it requires, plus td_radio and the
    # net-v2 service, td_netlink.
    set(COMPONENTS main td_radio td_netlink)
endif()

if(TD_QEMU_TEST AND NOT TD_RADIO_BUILD)
    message(FATAL_ERROR "TD_QEMU_TEST is a variant of the radio build: add -DTD_RADIO_BUILD=1")
endif()

include($ENV{IDF_PATH}/tools/cmake/project.cmake)
# Only build components that main/ (and its dependencies) ask for.
if(NOT TD_RADIO_BUILD)
    idf_build_set_property(MINIMAL_BUILD ON)
endif()
project(touchdeck)
