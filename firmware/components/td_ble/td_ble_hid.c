#include "td_ble_hid.h"
#include <string.h>

/* HID 1.11 short items. Every extent that sets a logical range is written so it stays positive when read as signed:
 * one byte only up to 0x7F, two bytes up to 0x7FFF. */
const uint8_t td_ble_report_map[] = {
    0x05, 0x01,       /* Usage Page (Generic Desktop) */
    0x09, 0x06,       /* Usage (Keyboard) */
    0xA1, 0x01,       /* Collection (Application) */
    0x85, TD_BLE_REPORT_KEYBOARD, /* Report ID (1) */
    0x05, 0x07,       /*   Usage Page (Keyboard/Keypad) */
    0x19, 0xE0,       /*   Usage Minimum (Left Control) */
    0x29, 0xE7,       /*   Usage Maximum (Right GUI) */
    0x15, 0x00,       /*   Logical Minimum (0) */
    0x25, 0x01,       /*   Logical Maximum (1) */
    0x75, 0x01,       /*   Report Size (1) */
    0x95, 0x08,       /*   Report Count (8) */
    0x81, 0x02,       /*   Input (Data, Variable, Absolute): the modifier bits */
    0x75, 0x08,       /*   Report Size (8) */
    0x95, 0x01,       /*   Report Count (1) */
    0x81, 0x01,       /*   Input (Constant): the reserved byte */
    0x05, 0x08,       /*   Usage Page (LEDs) */
    0x19, 0x01,       /*   Usage Minimum (Num Lock) */
    0x29, 0x05,       /*   Usage Maximum (Kana) */
    0x75, 0x01,       /*   Report Size (1) */
    0x95, 0x05,       /*   Report Count (5) */
    0x91, 0x02,       /*   Output (Data, Variable, Absolute): the LEDs */
    0x75, 0x03,       /*   Report Size (3) */
    0x95, 0x01,       /*   Report Count (1) */
    0x91, 0x01,       /*   Output (Constant): padding */
    0x05, 0x07,       /*   Usage Page (Keyboard/Keypad) */
    0x19, 0x00,       /*   Usage Minimum (0) */
    0x29, 0xFF,       /*   Usage Maximum (255) */
    0x15, 0x00,       /*   Logical Minimum (0) */
    0x26, 0xFF, 0x00, /*   Logical Maximum (255), two bytes so it stays positive */
    0x75, 0x08,       /*   Report Size (8) */
    0x95, 0x06,       /*   Report Count (6) */
    0x81, 0x00,       /*   Input (Data, Array): six key slots */
    0xC0,             /* End Collection */
    0x05, 0x0C,       /* Usage Page (Consumer) */
    0x09, 0x01,       /* Usage (Consumer Control) */
    0xA1, 0x01,       /* Collection (Application) */
    0x85, TD_BLE_REPORT_CONSUMER, /* Report ID (2) */
    0x15, 0x00,       /*   Logical Minimum (0) */
    0x26, 0xFF, 0x03, /*   Logical Maximum (0x03FF) */
    0x19, 0x00,       /*   Usage Minimum (0) */
    0x2A, 0xFF, 0x03, /*   Usage Maximum (0x03FF) */
    0x75, 0x10,       /*   Report Size (16) */
    0x95, 0x01,       /*   Report Count (1) */
    0x81, 0x00,       /*   Input (Data, Array): one usage */
    0xC0,             /* End Collection */
};
const size_t td_ble_report_map_len = sizeof td_ble_report_map;

const td_ble_report_t td_ble_reports[] = {
    { TD_BLE_REPORT_KEYBOARD, TD_BLE_REPORT_INPUT, TD_BLE_KEYBOARD_BYTES },
    { TD_BLE_REPORT_KEYBOARD, TD_BLE_REPORT_OUTPUT, TD_BLE_LED_BYTES },
    { TD_BLE_REPORT_CONSUMER, TD_BLE_REPORT_INPUT, TD_BLE_CONSUMER_BYTES },
};
const size_t td_ble_report_count = sizeof td_ble_reports / sizeof td_ble_reports[0];

/* An encrypted link whose keys came from an authenticated pairing, the passkey shown on the Device. */
#define HID_ENC (TD_BLE_F_ENC | TD_BLE_F_AUTHEN)

const td_ble_chr_t td_ble_chrs[] = {
    /* HID Service. Every characteristic needs an encrypted, authenticated, bonded link (HOGP 6.1 asks for encryption;
     * authentication keeps a silent Just Works bond or a man in the middle out). */
    { TD_BLE_SVC_HID, 0x2A4A, TD_BLE_ATTR_HID_INFO, TD_BLE_F_READ | HID_ENC, 0, 0 },
    { TD_BLE_SVC_HID, 0x2A4B, TD_BLE_ATTR_REPORT_MAP, TD_BLE_F_READ | HID_ENC, 0, 0 },
    { TD_BLE_SVC_HID, 0x2A4C, TD_BLE_ATTR_CONTROL_POINT, TD_BLE_F_WRITE_NR | HID_ENC, 0, 0 },
    { TD_BLE_SVC_HID, 0x2A4E, TD_BLE_ATTR_PROTOCOL_MODE, TD_BLE_F_READ | TD_BLE_F_WRITE_NR | HID_ENC, 0, 0 },
    { TD_BLE_SVC_HID, 0x2A4D, TD_BLE_ATTR_KEYBOARD_IN, TD_BLE_F_READ | TD_BLE_F_NOTIFY | HID_ENC,
      TD_BLE_REPORT_KEYBOARD, TD_BLE_REPORT_INPUT },
    { TD_BLE_SVC_HID, 0x2A4D, TD_BLE_ATTR_CONSUMER_IN, TD_BLE_F_READ | TD_BLE_F_NOTIFY | HID_ENC,
      TD_BLE_REPORT_CONSUMER, TD_BLE_REPORT_INPUT },
    { TD_BLE_SVC_HID, 0x2A4D, TD_BLE_ATTR_LED_OUT,
      TD_BLE_F_READ | TD_BLE_F_WRITE | TD_BLE_F_WRITE_NR | HID_ENC, TD_BLE_REPORT_KEYBOARD, TD_BLE_REPORT_OUTPUT },
    { TD_BLE_SVC_HID, 0x2A22, TD_BLE_ATTR_BOOT_KEYBOARD_IN, TD_BLE_F_READ | TD_BLE_F_NOTIFY | HID_ENC, 0, 0 },
    { TD_BLE_SVC_HID, 0x2A32, TD_BLE_ATTR_BOOT_KEYBOARD_OUT,
      TD_BLE_F_READ | TD_BLE_F_WRITE | TD_BLE_F_WRITE_NR | HID_ENC, 0, 0 },
    /* Battery Service, which HOGP requires. The level is fixed, and only a bonded, authenticated host reads it. */
    { TD_BLE_SVC_BATTERY, 0x2A19, TD_BLE_ATTR_BATTERY_LEVEL, TD_BLE_F_READ | HID_ENC, 0, 0 },
    /* Device Information: the PnP ID only. */
    { TD_BLE_SVC_DEVICE_INFO, 0x2A50, TD_BLE_ATTR_PNP_ID, TD_BLE_F_READ, 0, 0 },
};
const size_t td_ble_chr_count = sizeof td_ble_chrs / sizeof td_ble_chrs[0];

const uint8_t td_ble_hid_info[4] = { 0x11, 0x01, 0x00, 0x02 };
const uint8_t td_ble_pnp_id[7] = { 0x01, 0xE5, 0x02, 0x00, 0x00, TD_BLE_GATT_VERSION & 0xFF,
                                   (TD_BLE_GATT_VERSION >> 8) & 0xFF };

size_t td_ble_encode(const key_report_t *report, uint8_t protocol, uint8_t out[TD_BLE_KEYBOARD_BYTES],
                     td_ble_input_t *input)
{
    if (report->type == KEY_REPORT_CON) {
        if (protocol == 0 || report->con.usage > TD_BLE_CONSUMER_MAX) {
            return 0;
        }
        out[0] = (uint8_t)(report->con.usage & 0xFF);
        out[1] = (uint8_t)(report->con.usage >> 8);
        *input = TD_BLE_IN_CONSUMER;
        return TD_BLE_CONSUMER_BYTES;
    }
    out[0] = report->kbd.mods;
    out[1] = 0;
    memcpy(out + 2, report->kbd.keys, KEY_REPORT_KEYS);
    *input = protocol == 0 ? TD_BLE_IN_BOOT_KEYBOARD : TD_BLE_IN_KEYBOARD;
    return TD_BLE_KEYBOARD_BYTES;
}

uint32_t td_ble_gatt_flags(uint8_t flags)
{
    const bool enc = flags & (TD_BLE_F_ENC | TD_BLE_F_AUTHEN);
    const bool authen = flags & TD_BLE_F_AUTHEN;
    const uint32_t read = (enc ? TD_BLE_GATT_F_READ_ENC : 0) | (authen ? TD_BLE_GATT_F_READ_AUTHEN : 0);
    const uint32_t write = (enc ? TD_BLE_GATT_F_WRITE_ENC : 0) | (authen ? TD_BLE_GATT_F_WRITE_AUTHEN : 0);
    const uint32_t notify = (enc ? TD_BLE_GATT_F_NOTIFY_ENC : 0) | (authen ? TD_BLE_GATT_F_NOTIFY_AUTHEN : 0);
    uint32_t out = 0;
    if (flags & TD_BLE_F_READ) out |= TD_BLE_GATT_F_READ | read;
    if (flags & TD_BLE_F_WRITE) out |= TD_BLE_GATT_F_WRITE | write;
    if (flags & TD_BLE_F_WRITE_NR) out |= TD_BLE_GATT_F_WRITE_NO_RSP | write;
    if (flags & TD_BLE_F_NOTIFY) out |= TD_BLE_GATT_F_NOTIFY | notify;
    return out;
}

uint8_t td_ble_report_ref_flags(uint8_t flags)
{
    return (uint8_t)(TD_BLE_ATT_F_READ | ((flags & (TD_BLE_F_ENC | TD_BLE_F_AUTHEN)) ? TD_BLE_ATT_F_READ_ENC : 0) |
                     ((flags & TD_BLE_F_AUTHEN) ? TD_BLE_ATT_F_READ_AUTHEN : 0));
}

static uint32_t fnv(uint32_t hash, const void *data, size_t len)
{
    const uint8_t *bytes = data;
    for (size_t i = 0; i < len; i++) {
        hash = (hash ^ bytes[i]) * 16777619u;
    }
    return hash;
}

uint32_t td_ble_gatt_fingerprint(void)
{
    uint32_t hash = fnv(2166136261u, td_ble_report_map, td_ble_report_map_len);
    for (size_t i = 0; i < td_ble_chr_count; i++) {
        const td_ble_chr_t *c = &td_ble_chrs[i];
        const uint8_t fields[] = { (uint8_t)(c->service >> 8), (uint8_t)c->service, (uint8_t)(c->uuid >> 8),
                                   (uint8_t)c->uuid, (uint8_t)c->attr, c->flags, c->report_id, c->report_type };
        hash = fnv(hash, fields, sizeof fields);
    }
    hash = fnv(hash, td_ble_hid_info, sizeof td_ble_hid_info);
    return hash;
}
