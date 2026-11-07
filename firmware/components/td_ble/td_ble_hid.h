#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "key_sink.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * What the Device shows a Bluetooth host as a HID-over-GATT keyboard: the report map, the reports, and the GATT
 * characteristics of the HID, Battery and Device Information services. Plain data and pure functions, so the host
 * tests check exactly what the radio build registers. td_ble_nimble.c turns td_ble_chrs into NimBLE's tables.
 *
 * Any change here that a bonded host could have cached (the report map, a characteristic, its flags or a Report
 * Reference) must bump TD_BLE_GATT_VERSION. The version is the PnP ID's product version, and a Device that boots
 * with a new version raises Service Changed for its bonded hosts. firmware/tools/host/ble_hid_test.c pins the
 * layout to the version, so a change without a bump fails the host tests. The archived M3 build used version 1.
 */
#define TD_BLE_GATT_VERSION 2

/* Report IDs and types, as the report map and the Report Reference descriptors give them. */
#define TD_BLE_REPORT_KEYBOARD 1
#define TD_BLE_REPORT_CONSUMER 2
#define TD_BLE_REPORT_INPUT 1
#define TD_BLE_REPORT_OUTPUT 2

/* The keyboard report is the boot keyboard report: modifiers, a reserved byte and six key slots. The LED output report
 * is one byte. The consumer report is one 16-bit usage, little-endian. No report value carries its ID byte. */
#define TD_BLE_KEYBOARD_BYTES 8
#define TD_BLE_LED_BYTES 1
#define TD_BLE_CONSUMER_BYTES 2
/* The consumer report's logical and usage maximum. Usages above it can't be sent; the report map's 2-byte extent
 * 0x03FF stays positive under the HID 1.11 signed rule, unlike the archived 0xFFFF that read as -1. */
#define TD_BLE_CONSUMER_MAX 0x03FF

/* The Battery Level the Device reports. The board can't measure a battery and runs from USB, so it is fixed. */
#define TD_BLE_BATTERY_LEVEL 100

extern const uint8_t td_ble_report_map[];
extern const size_t td_ble_report_map_len;

typedef struct {
    uint8_t id;
    uint8_t type;
    uint8_t bytes;
} td_ble_report_t;

/* Every report the map declares, with the length its value has on the air. */
extern const td_ble_report_t td_ble_reports[];
extern const size_t td_ble_report_count;

/* The services, as 16-bit UUIDs. */
#define TD_BLE_SVC_HID 0x1812
#define TD_BLE_SVC_BATTERY 0x180F
#define TD_BLE_SVC_DEVICE_INFO 0x180A

typedef enum {
    TD_BLE_ATTR_REPORT_MAP,
    TD_BLE_ATTR_HID_INFO,
    TD_BLE_ATTR_CONTROL_POINT,
    TD_BLE_ATTR_PROTOCOL_MODE,
    TD_BLE_ATTR_KEYBOARD_IN,
    TD_BLE_ATTR_CONSUMER_IN,
    TD_BLE_ATTR_LED_OUT,
    TD_BLE_ATTR_BOOT_KEYBOARD_IN,
    TD_BLE_ATTR_BOOT_KEYBOARD_OUT,
    TD_BLE_ATTR_BATTERY_LEVEL,
    TD_BLE_ATTR_PNP_ID,
    TD_BLE_ATTR_COUNT,
} td_ble_attr_t;

/* Characteristic properties. ENC means reading, writing and turning notifications on all need an encrypted link with
 * a bonded host; the access callback refuses an encrypted link without a bond too. */
#define TD_BLE_F_READ 0x01
#define TD_BLE_F_WRITE 0x02
#define TD_BLE_F_WRITE_NR 0x04
#define TD_BLE_F_NOTIFY 0x08
#define TD_BLE_F_ENC 0x10

typedef struct {
    uint16_t service;
    uint16_t uuid;
    td_ble_attr_t attr;
    uint8_t flags;
    /* A Report characteristic's Report Reference: its ID and type. Zero for every other characteristic. */
    uint8_t report_id;
    uint8_t report_type;
} td_ble_chr_t;

/* In service order: HID, Battery, Device Information. Each service's characteristics are contiguous. */
extern const td_ble_chr_t td_ble_chrs[];
extern const size_t td_ble_chr_count;

/* HID Information: bcdHID 1.11, country 0, flags NormallyConnectable. */
extern const uint8_t td_ble_hid_info[4];
/* PnP ID: vendor ID source Bluetooth SIG, Espressif's company ID, product 0, version TD_BLE_GATT_VERSION. */
extern const uint8_t td_ble_pnp_id[7];

/* The input report characteristics a report can go out on. */
typedef enum {
    TD_BLE_IN_KEYBOARD,
    TD_BLE_IN_CONSUMER,
    TD_BLE_IN_BOOT_KEYBOARD,
    TD_BLE_IN_COUNT,
} td_ble_input_t;

/*
 * The bytes a key report sends in the given protocol mode (0 boot, 1 report) and the characteristic they go out on.
 * Returns the length, or 0 when the report has nothing to send there: a consumer report in boot protocol, or a
 * consumer usage above TD_BLE_CONSUMER_MAX.
 */
size_t td_ble_encode(const key_report_t *report, uint8_t protocol, uint8_t out[TD_BLE_KEYBOARD_BYTES],
                     td_ble_input_t *input);

/* A 32-bit FNV-1a fingerprint of the report map and the GATT table, which the host tests pin to the version. */
uint32_t td_ble_gatt_fingerprint(void);

#ifdef __cplusplus
}
#endif
