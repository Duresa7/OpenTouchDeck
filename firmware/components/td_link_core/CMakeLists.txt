idf_component_register(SRCS "td_input.c" "td_v2_json.c" "td_frame.c" "td_crc16.c" "td_xfer.c" "td_transfer.c"
                       INCLUDE_DIRS "include")

if(CMAKE_PROJECT_NAME STREQUAL "touchdeck_test")
    target_add_binary_data(${COMPONENT_LIB}
                           "${CMAKE_CURRENT_LIST_DIR}/../../../schema/protocol/golden-frames.json" TEXT)
    target_add_binary_data(${COMPONENT_LIB}
                           "${CMAKE_CURRENT_LIST_DIR}/../../../schema/protocol/golden-frames-v2.json" TEXT)
endif()
