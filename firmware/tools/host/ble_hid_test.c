/*
 * The Bluetooth keyboard's GATT table and report map, as the radio build registers them (td_ble_hid.c). The report map
 * is decoded item by item and checked against the rules from the crash analysis in #275: Report IDs nonzero and unique
 * per type, every Logical Minimum at most its Maximum under the signed rule, each report's length equal to what is
 * sent, each report on exactly one Report characteristic, at most 512 bytes. The GATT table must encrypt every HID
 * characteristic, carry a Battery Service, and match the version it is pinned to.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "td_ble_hid.h"

static int g_failures;
/* Set while a check is expected to fail, so its message doesn't read as a failure. */
static bool g_quiet;

#define CHECK(cond, ...)                                                                                               \
    do {                                                                                                               \
        if (!(cond)) {                                                                                                 \
            if (!g_quiet) {                                                                                            \
                printf("FAIL %s:%d: ", __FILE__, __LINE__);                                                            \
                printf(__VA_ARGS__);                                                                                   \
                printf("\n");                                                                                          \
            }                                                                                                          \
            g_failures++;                                                                                              \
        }                                                                                                              \
    } while (0)

/* ---- A HID 1.11 report descriptor decoder, short items only. ---- */

#define MAX_REPORTS 16

typedef struct {
    uint8_t id;
    uint8_t type; /* TD_BLE_REPORT_INPUT, _OUTPUT, or 3 for Feature */
    unsigned bits;
} decoded_report_t;

typedef struct {
    decoded_report_t reports[MAX_REPORTS];
    size_t count;
    unsigned collections_opened;
    unsigned application_collections;
    int depth;
    bool used_id;
    bool error;
} decoded_t;

static int32_t signed_value(const uint8_t *data, unsigned size)
{
    switch (size) {
    case 1: return (int8_t)data[0];
    case 2: return (int16_t)(data[0] | data[1] << 8);
    case 4: return (int32_t)((uint32_t)data[0] | (uint32_t)data[1] << 8 | (uint32_t)data[2] << 16 |
                             (uint32_t)data[3] << 24);
    default: return 0;
    }
}

static uint32_t unsigned_value(const uint8_t *data, unsigned size)
{
    uint32_t v = 0;
    for (unsigned i = 0; i < size; i++) {
        v |= (uint32_t)data[i] << (8 * i);
    }
    return v;
}

static decoded_report_t *report_for(decoded_t *d, uint8_t id, uint8_t type)
{
    for (size_t i = 0; i < d->count; i++) {
        if (d->reports[i].id == id && d->reports[i].type == type) {
            return &d->reports[i];
        }
    }
    if (d->count == MAX_REPORTS) {
        d->error = true;
        return NULL;
    }
    d->reports[d->count] = (decoded_report_t){ .id = id, .type = type };
    return &d->reports[d->count++];
}

static decoded_t decode(const uint8_t *map, size_t len)
{
    decoded_t d = { 0 };
    /* Globals. */
    uint8_t report_id = 0;
    uint32_t report_size = 0;
    uint32_t report_count = 0;
    int32_t logical_min = 0;
    int32_t logical_max = 0;
    bool have_min = false;
    bool have_max = false;
    /* The report IDs each application collection uses, so an ID can't belong to two of them. */
    uint8_t collection_of_id[256] = { 0 };
    size_t i = 0;
    while (i < len) {
        const uint8_t prefix = map[i++];
        CHECK(prefix != 0xFE, "long item at %u", (unsigned)(i - 1));
        if (prefix == 0xFE) {
            d.error = true;
            break;
        }
        const unsigned size = (prefix & 3) == 3 ? 4 : prefix & 3;
        const unsigned type = (prefix >> 2) & 3;
        const unsigned tag = prefix >> 4;
        if (i + size > len) {
            CHECK(false, "item at %u runs past the end", (unsigned)(i - 1));
            d.error = true;
            break;
        }
        const uint8_t *data = map + i;
        i += size;
        if (type == 0) { /* Main */
            if (tag == 0x8 || tag == 0x9 || tag == 0xB) {
                const uint8_t rtype = tag == 0x8 ? TD_BLE_REPORT_INPUT : tag == 0x9 ? TD_BLE_REPORT_OUTPUT : 3;
                CHECK(have_min && have_max, "main item without a logical extent at %u", (unsigned)(i - size - 1));
                CHECK(logical_min <= logical_max, "Logical Minimum %ld > Logical Maximum %ld (signed) for ID %u",
                      (long)logical_min, (long)logical_max, report_id);
                decoded_report_t *r = report_for(&d, report_id, rtype);
                if (r) {
                    r->bits += report_size * report_count;
                }
                if (report_id) {
                    d.used_id = true;
                    CHECK(collection_of_id[report_id] == 0 || collection_of_id[report_id] == d.application_collections,
                          "report ID %u used in two application collections", report_id);
                    collection_of_id[report_id] = (uint8_t)d.application_collections;
                }
            } else if (tag == 0xA) { /* Collection */
                d.collections_opened++;
                if (d.depth == 0) {
                    CHECK(size == 1 && data[0] == 0x01, "a top-level collection that isn't Application");
                    d.application_collections++;
                }
                d.depth++;
            } else if (tag == 0xC) { /* End Collection */
                d.depth--;
                CHECK(d.depth >= 0, "End Collection without a Collection");
            } else {
                CHECK(false, "unknown main item tag 0x%x", tag);
            }
        } else if (type == 1) { /* Global */
            switch (tag) {
            case 0x0: break; /* Usage Page */
            case 0x1:
                logical_min = signed_value(data, size);
                have_min = true;
                break;
            case 0x2:
                logical_max = signed_value(data, size);
                have_max = true;
                break;
            case 0x7: report_size = unsigned_value(data, size); break;
            case 0x8:
                CHECK(size == 1, "Report ID of %u bytes", size);
                report_id = size ? data[0] : 0;
                CHECK(report_id != 0, "Report ID 0");
                break;
            case 0x9: report_count = unsigned_value(data, size); break;
            default: CHECK(false, "unexpected global item tag 0x%x", tag); break;
            }
        } else if (type == 2) { /* Local: Usage, Usage Minimum, Usage Maximum */
            CHECK(tag <= 2, "unexpected local item tag 0x%x", tag);
        } else {
            CHECK(false, "reserved item type");
        }
    }
    CHECK(d.depth == 0, "%d collections left open", d.depth);
    return d;
}

static void test_report_map(void)
{
    CHECK(td_ble_report_map_len <= 512, "report map is %u bytes, over 512", (unsigned)td_ble_report_map_len);
    const decoded_t d = decode(td_ble_report_map, td_ble_report_map_len);
    CHECK(!d.error, "the report map did not decode");
    CHECK(d.used_id, "the map uses no Report ID, but the HID service carries two application reports");
    CHECK(d.application_collections == 2, "%u application collections, want keyboard and consumer",
          d.application_collections);
    CHECK(d.count == td_ble_report_count, "map declares %u reports, td_ble_reports lists %u", (unsigned)d.count,
          (unsigned)td_ble_report_count);
    for (size_t i = 0; i < d.count; i++) {
        const decoded_report_t *r = &d.reports[i];
        CHECK(r->id != 0, "a report without an ID");
        CHECK(r->type != 3, "a Feature report, which no characteristic carries");
        CHECK(r->bits % 8 == 0, "report %u/%u is %u bits, not whole bytes", r->id, r->type, r->bits);
        bool listed = false;
        for (size_t j = 0; j < td_ble_report_count; j++) {
            if (td_ble_reports[j].id == r->id && td_ble_reports[j].type == r->type) {
                listed = true;
                CHECK(td_ble_reports[j].bytes * 8u == r->bits, "report %u/%u: map has %u bits, td_ble_reports %u bytes",
                      r->id, r->type, r->bits, td_ble_reports[j].bytes);
            }
        }
        CHECK(listed, "report %u/%u is in the map but not in td_ble_reports", r->id, r->type);
    }
    for (size_t i = 0; i < td_ble_report_count; i++) {
        for (size_t j = i + 1; j < td_ble_report_count; j++) {
            CHECK(td_ble_reports[i].id != td_ble_reports[j].id || td_ble_reports[i].type != td_ble_reports[j].type,
                  "report %u/%u listed twice", td_ble_reports[i].id, td_ble_reports[i].type);
        }
    }
}

/* The consumer report's extent must stay positive: the archived map's 2-byte 0xFFFF reads as -1. */
static void test_signed_rule_catches_the_old_consumer_extent(void)
{
    static const uint8_t old_map[] = { 0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x02, 0x15, 0x00, 0x26, 0xFF, 0xFF,
                                       0x19, 0x00, 0x2A, 0xFF, 0xFF, 0x75, 0x10, 0x95, 0x01, 0x81, 0x00, 0xC0 };
    const int before = g_failures;
    g_quiet = true;
    decode(old_map, sizeof old_map);
    g_quiet = false;
    const bool caught = g_failures > before;
    g_failures = before;
    CHECK(caught, "the decoder accepted Logical Maximum 0xFFFF in 2 bytes");
}

static const td_ble_chr_t *find_chr(uint16_t uuid, uint8_t id, uint8_t type)
{
    const td_ble_chr_t *found = NULL;
    for (size_t i = 0; i < td_ble_chr_count; i++) {
        if (td_ble_chrs[i].uuid == uuid && td_ble_chrs[i].report_id == id && td_ble_chrs[i].report_type == type) {
            found = &td_ble_chrs[i];
        }
    }
    return found;
}

static void test_gatt_table(void)
{
    /* Each report on exactly one Report characteristic, and every Report characteristic on a report. */
    for (size_t i = 0; i < td_ble_report_count; i++) {
        unsigned n = 0;
        for (size_t c = 0; c < td_ble_chr_count; c++) {
            n += td_ble_chrs[c].uuid == 0x2A4D && td_ble_chrs[c].report_id == td_ble_reports[i].id &&
                 td_ble_chrs[c].report_type == td_ble_reports[i].type;
        }
        CHECK(n == 1, "report %u/%u is on %u Report characteristics", td_ble_reports[i].id, td_ble_reports[i].type, n);
    }
    bool seen_attr[TD_BLE_ATTR_COUNT] = { false };
    bool battery = false;
    for (size_t c = 0; c < td_ble_chr_count; c++) {
        const td_ble_chr_t *chr = &td_ble_chrs[c];
        CHECK(chr->attr < TD_BLE_ATTR_COUNT && !seen_attr[chr->attr], "attribute %u twice", (unsigned)chr->attr);
        if (chr->attr < TD_BLE_ATTR_COUNT) {
            seen_attr[chr->attr] = true;
        }
        if (c > 0 && chr->service != td_ble_chrs[c - 1].service) {
            for (size_t e = 0; e + 1 < c; e++) {
                CHECK(td_ble_chrs[e].service != chr->service, "service 0x%04x is not contiguous", chr->service);
            }
        }
        if (chr->uuid == 0x2A4D) {
            bool listed = false;
            for (size_t i = 0; i < td_ble_report_count; i++) {
                listed |= td_ble_reports[i].id == chr->report_id && td_ble_reports[i].type == chr->report_type;
            }
            CHECK(listed, "Report characteristic %u/%u has no report in the map", chr->report_id, chr->report_type);
        } else {
            CHECK(chr->report_id == 0 && chr->report_type == 0, "a Report Reference on 0x%04x", chr->uuid);
        }
        if (chr->service == TD_BLE_SVC_HID) {
            /* HOGP 6.1: every HID Service characteristic needs encryption. */
            CHECK(chr->flags & TD_BLE_F_ENC, "HID characteristic 0x%04x is not encrypted", chr->uuid);
        }
        if (chr->flags & TD_BLE_F_NOTIFY) {
            CHECK(chr->uuid == 0x2A22 || (chr->uuid == 0x2A4D && chr->report_type == TD_BLE_REPORT_INPUT),
                  "0x%04x notifies but is no input report", chr->uuid);
        }
        if (chr->service == TD_BLE_SVC_BATTERY && chr->uuid == 0x2A19) {
            battery = true;
            CHECK(chr->flags & TD_BLE_F_READ, "Battery Level is not readable");
        }
    }
    for (unsigned a = 0; a < TD_BLE_ATTR_COUNT; a++) {
        CHECK(seen_attr[a], "attribute %u has no characteristic", a);
    }
    CHECK(battery, "no Battery Service with a Battery Level (HOGP 3)");
    CHECK(TD_BLE_BATTERY_LEVEL <= 100, "Battery Level %u over 100", TD_BLE_BATTERY_LEVEL);
    CHECK(find_chr(0x2A4B, 0, 0) && find_chr(0x2A4A, 0, 0) && find_chr(0x2A4C, 0, 0) && find_chr(0x2A4E, 0, 0),
          "HID service misses Report Map, HID Information, Control Point or Protocol Mode");
    CHECK(find_chr(0x2A22, 0, 0) && find_chr(0x2A32, 0, 0), "boot keyboard reports missing");
    CHECK(td_ble_hid_info[0] == 0x11 && td_ble_hid_info[1] == 0x01, "bcdHID is not 1.11");
    CHECK(td_ble_hid_info[3] & 0x02, "HID Information does not say NormallyConnectable");
    CHECK(td_ble_pnp_id[5] == (TD_BLE_GATT_VERSION & 0xFF) && td_ble_pnp_id[6] == (TD_BLE_GATT_VERSION >> 8),
          "the PnP ID's product version is not TD_BLE_GATT_VERSION");
}

static size_t report_bytes(uint8_t id, uint8_t type)
{
    for (size_t i = 0; i < td_ble_report_count; i++) {
        if (td_ble_reports[i].id == id && td_ble_reports[i].type == type) {
            return td_ble_reports[i].bytes;
        }
    }
    return 0;
}

static void test_encode_lengths(void)
{
    uint8_t out[TD_BLE_KEYBOARD_BYTES];
    td_ble_input_t input;
    key_report_t kbd = key_report_kbd(0x02, 0x04);
    size_t len = td_ble_encode(&kbd, 1, out, &input);
    CHECK(input == TD_BLE_IN_KEYBOARD && len == report_bytes(TD_BLE_REPORT_KEYBOARD, TD_BLE_REPORT_INPUT),
          "keyboard report: input %d, %u bytes", input, (unsigned)len);
    CHECK(out[0] == 0x02 && out[1] == 0 && out[2] == 0x04, "keyboard report carries a Report ID byte or wrong bytes");
    len = td_ble_encode(&kbd, 0, out, &input);
    CHECK(input == TD_BLE_IN_BOOT_KEYBOARD && len == 8, "boot keyboard report: input %d, %u bytes", input,
          (unsigned)len);
    key_report_t con = key_report_con(0x00E9);
    len = td_ble_encode(&con, 1, out, &input);
    CHECK(input == TD_BLE_IN_CONSUMER && len == report_bytes(TD_BLE_REPORT_CONSUMER, TD_BLE_REPORT_INPUT),
          "consumer report: input %d, %u bytes", input, (unsigned)len);
    CHECK(out[0] == 0xE9 && out[1] == 0x00, "consumer report is not the little-endian usage");
    CHECK(td_ble_encode(&con, 0, out, &input) == 0, "a consumer report went out in boot protocol");
    key_report_t big = key_report_con(TD_BLE_CONSUMER_MAX + 1);
    CHECK(td_ble_encode(&big, 1, out, &input) == 0, "a consumer usage above the logical maximum went out");
    key_report_t max = key_report_con(TD_BLE_CONSUMER_MAX);
    CHECK(td_ble_encode(&max, 1, out, &input) == 2, "the largest consumer usage did not go out");
    CHECK(report_bytes(TD_BLE_REPORT_KEYBOARD, TD_BLE_REPORT_OUTPUT) == TD_BLE_LED_BYTES, "LED report is not 1 byte");
}

/*
 * Every HID Service characteristic (Report Map, Reports, Protocol Mode, Control Point, HID Information, the boot
 * reports) and the Battery Level needs an encrypted link whose keys came from an authenticated pairing, on every way in
 * NimBLE offers: read, write, write without response, and turning notifications on. Only the PnP ID stays open.
 */
static void test_every_hid_characteristic_needs_authentication(void)
{
    unsigned hid = 0;
    for (size_t c = 0; c < td_ble_chr_count; c++) {
        const td_ble_chr_t *chr = &td_ble_chrs[c];
        const uint32_t f = td_ble_gatt_flags(chr->flags);
        const bool guarded = chr->service == TD_BLE_SVC_HID || chr->service == TD_BLE_SVC_BATTERY;
        if (!guarded) {
            CHECK(chr->uuid == 0x2A50, "0x%04x outside the HID and Battery services", chr->uuid);
            CHECK(!(chr->flags & (TD_BLE_F_WRITE | TD_BLE_F_WRITE_NR | TD_BLE_F_NOTIFY)), "PnP ID is not read-only");
            continue;
        }
        hid += chr->service == TD_BLE_SVC_HID;
        CHECK((chr->flags & (TD_BLE_F_ENC | TD_BLE_F_AUTHEN)) == (TD_BLE_F_ENC | TD_BLE_F_AUTHEN),
              "0x%04x does not need an authenticated link", chr->uuid);
        if (chr->flags & TD_BLE_F_READ) {
            CHECK((f & (TD_BLE_GATT_F_READ | TD_BLE_GATT_F_READ_ENC | TD_BLE_GATT_F_READ_AUTHEN)) ==
                      (TD_BLE_GATT_F_READ | TD_BLE_GATT_F_READ_ENC | TD_BLE_GATT_F_READ_AUTHEN),
                  "0x%04x reads without authentication: 0x%05lx", chr->uuid, (unsigned long)f);
        }
        if (chr->flags & (TD_BLE_F_WRITE | TD_BLE_F_WRITE_NR)) {
            CHECK((f & (TD_BLE_GATT_F_WRITE_ENC | TD_BLE_GATT_F_WRITE_AUTHEN)) ==
                      (TD_BLE_GATT_F_WRITE_ENC | TD_BLE_GATT_F_WRITE_AUTHEN),
                  "0x%04x writes without authentication: 0x%05lx", chr->uuid, (unsigned long)f);
        }
        if (chr->flags & TD_BLE_F_NOTIFY) {
            CHECK((f & (TD_BLE_GATT_F_NOTIFY | TD_BLE_GATT_F_NOTIFY_ENC | TD_BLE_GATT_F_NOTIFY_AUTHEN)) ==
                      (TD_BLE_GATT_F_NOTIFY | TD_BLE_GATT_F_NOTIFY_ENC | TD_BLE_GATT_F_NOTIFY_AUTHEN),
                  "0x%04x notifies without authentication: 0x%05lx", chr->uuid, (unsigned long)f);
        }
        if (chr->report_id) {
            CHECK(td_ble_report_ref_flags(chr->flags) ==
                      (TD_BLE_ATT_F_READ | TD_BLE_ATT_F_READ_ENC | TD_BLE_ATT_F_READ_AUTHEN),
                  "the Report Reference of %u/%u reads without authentication", chr->report_id, chr->report_type);
        }
    }
    CHECK(hid == 9, "%u HID Service characteristics, expected 9", hid);
    /* The mapping itself: each property gets exactly its own security bits. */
    CHECK(td_ble_gatt_flags(TD_BLE_F_READ) == TD_BLE_GATT_F_READ, "an open read got security bits");
    CHECK(td_ble_gatt_flags(TD_BLE_F_READ | TD_BLE_F_ENC) == (TD_BLE_GATT_F_READ | TD_BLE_GATT_F_READ_ENC),
          "an encrypted read");
    CHECK(td_ble_gatt_flags(TD_BLE_F_WRITE_NR | TD_BLE_F_ENC | TD_BLE_F_AUTHEN) ==
              (TD_BLE_GATT_F_WRITE_NO_RSP | TD_BLE_GATT_F_WRITE_ENC | TD_BLE_GATT_F_WRITE_AUTHEN),
          "an authenticated write without response");
    CHECK(td_ble_report_ref_flags(TD_BLE_F_READ) == TD_BLE_ATT_F_READ, "an open Report Reference");
}

/*
 * The layout a bonded host may cache, pinned to the version that announced it. Changing the report map or the GATT
 * table changes the fingerprint: bump TD_BLE_GATT_VERSION and add a row here, so bonded hosts get Service Changed and
 * a new PnP version.
 */
static const struct {
    unsigned version;
    uint32_t fingerprint;
} PINS[] = {
    { 2, 0x020347aau },
    { 3, 0xb3dfd0fau },
};

static void test_version_pinned(void)
{
    const uint32_t fingerprint = td_ble_gatt_fingerprint();
    bool pinned = false;
    for (size_t i = 0; i < sizeof PINS / sizeof PINS[0]; i++) {
        if (PINS[i].version == TD_BLE_GATT_VERSION) {
            pinned = true;
            CHECK(PINS[i].fingerprint == fingerprint,
                  "GATT fingerprint 0x%08lx, version %u was 0x%08lx: bump TD_BLE_GATT_VERSION and pin the new one",
                  (unsigned long)fingerprint, TD_BLE_GATT_VERSION, (unsigned long)PINS[i].fingerprint);
        }
    }
    CHECK(pinned, "TD_BLE_GATT_VERSION %u has no pinned fingerprint (0x%08lx)", TD_BLE_GATT_VERSION,
          (unsigned long)fingerprint);
}

int main(void)
{
    test_report_map();
    test_signed_rule_catches_the_old_consumer_extent();
    test_gatt_table();
    test_encode_lengths();
    test_every_hid_characteristic_needs_authentication();
    test_version_pinned();
    if (g_failures) {
        printf("ble hid: %d failures\n", g_failures);
        return 1;
    }
    printf("ble hid: 6 tests passed\n");
    return 0;
}
