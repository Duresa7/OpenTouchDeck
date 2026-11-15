/*
 * NimBLE under the core: the GATT table built from td_ble_chrs, the GAP events, advertising, the store and the timer.
 * This is the only file that may start the controller or the host, advertise, or touch the accept list, and it does so
 * only when the core asks (td_ble_core.h). firmware/tools/test_radio_guard.py checks that no other source calls those
 * functions.
 *
 * Threading: every core call runs under s_mutex. The host task delivers GAP events, GATT access, the timer and the
 * drain; the td_link and console tasks run commands. NimBLE calls the application without its own lock held, so the
 * order is always s_mutex, then NimBLE's lock. The key sink runs on other tasks and touches only the report queue,
 * under s_queue_lock.
 */
#include "td_ble.h"
#include "td_ble_core.h"
#include "td_ble_hid.h"
#include "td_ble_service.h"
#include <string.h>
#include "esp_attr.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "host/ble_hs.h"
#include "host/util/util.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "nvs.h"
#include "nvs_flash.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"
#include "td_device.h"
#include "td_link.h"

void ble_store_config_init(void);
/* NimBLE defines these in ble_gap.c and ble_hs_pvcy.c without declaring them in a public header. */
void ble_gap_conn_foreach_handle(ble_gap_conn_foreach_handle_fn *cb, void *arg);
int ble_hs_pvcy_add_entry(const uint8_t *addr, uint8_t addr_type, const uint8_t *irk);
int ble_hs_pvcy_remove_entry(uint8_t addr_type, const uint8_t *addr);

/* td_ble_hid.c builds NimBLE's flags from its own copies of these values, so the host tests check them. */
_Static_assert(TD_BLE_GATT_F_READ == BLE_GATT_CHR_F_READ, "GATT flag");
_Static_assert(TD_BLE_GATT_F_WRITE_NO_RSP == BLE_GATT_CHR_F_WRITE_NO_RSP, "GATT flag");
_Static_assert(TD_BLE_GATT_F_WRITE == BLE_GATT_CHR_F_WRITE, "GATT flag");
_Static_assert(TD_BLE_GATT_F_NOTIFY == BLE_GATT_CHR_F_NOTIFY, "GATT flag");
_Static_assert(TD_BLE_GATT_F_READ_ENC == BLE_GATT_CHR_F_READ_ENC, "GATT flag");
_Static_assert(TD_BLE_GATT_F_READ_AUTHEN == BLE_GATT_CHR_F_READ_AUTHEN, "GATT flag");
_Static_assert(TD_BLE_GATT_F_WRITE_ENC == BLE_GATT_CHR_F_WRITE_ENC, "GATT flag");
_Static_assert(TD_BLE_GATT_F_WRITE_AUTHEN == BLE_GATT_CHR_F_WRITE_AUTHEN, "GATT flag");
_Static_assert(TD_BLE_GATT_F_NOTIFY_ENC == BLE_GATT_CHR_F_NOTIFY_INDICATE_ENC, "GATT flag");
_Static_assert(TD_BLE_GATT_F_NOTIFY_AUTHEN == BLE_GATT_CHR_F_NOTIFY_INDICATE_AUTHEN, "GATT flag");
_Static_assert(TD_BLE_ATT_F_READ == BLE_ATT_F_READ, "ATT flag");
_Static_assert(TD_BLE_ATT_F_READ_ENC == BLE_ATT_F_READ_ENC, "ATT flag");
_Static_assert(TD_BLE_ATT_F_READ_AUTHEN == BLE_ATT_F_READ_AUTHEN, "ATT flag");

static const char *TAG = "ble";

#define NVS_NAMESPACE "td_ble"
#define NVS_ENABLED "enabled"
#define NVS_GATT_VERSION "gatt_ver"
#define APPEARANCE_KEYBOARD 0x03C1
/* Advertising intervals in 0.625 ms units: pairing 30-50 ms, reconnect 20-30 ms, then 1-2.5 s (HOGP 5.1). */
#define PAIRING_ITVL_MIN 48
#define PAIRING_ITVL_MAX 80
#define FAST_ITVL_MIN 32
#define FAST_ITVL_MAX 48
#define SLOW_ITVL_MIN 1600
#define SLOW_ITVL_MAX 4000
/* The one connection-parameter request, in 1.25 ms and 10 ms units: 7.5-50 ms, latency 0, 4 s timeout. The radio
 * build's PPCP options hold the same values (radio/radio.defaults). */
#define CONN_ITVL_MIN 6
#define CONN_ITVL_MAX 40
#define CONN_LATENCY 0
#define CONN_TIMEOUT 400

/* Everything but the spinlock and the posting guard lives in PSRAM, so the radio build's static internal RAM barely
 * grows. PSRAM stays readable while flash is written (the radio build runs code from PSRAM). */
typedef struct {
    td_ble_core_t core;
    td_ble_port_t port;
    SemaphoreHandle_t mutex;
    struct ble_npl_callout timer;
    struct ble_npl_event drain;
    uint8_t own_addr_type;
    uint16_t handles[TD_BLE_ATTR_COUNT];
    struct ble_gatt_svc_def *services;
    char name[TD_DEVICE_NAME_MAX + 1];
} ble_t;

static ble_t *s;
static portMUX_TYPE s_queue_lock = portMUX_INITIALIZER_UNLOCKED;
/* In PSRAM like the rest of the state, so the hook costs no static internal RAM. */
static EXT_RAM_BSS_ATTR td_ble_passkey_hook_t s_passkey_hook;
/* The key sink posts the drain event only while the host's event queue exists; a stop waits for posts in flight. */
static bool s_post_ok;
static int s_posting;

static int gap_event(struct ble_gap_event *event, void *arg);

static void lock(void)
{
    xSemaphoreTake(s->mutex, portMAX_DELAY);
}

static void unlock(void)
{
    xSemaphoreGive(s->mutex);
}

/* ---- The port. ---- */

static bool port_load_enabled(void *ctx)
{
    (void)ctx;
    nvs_handle_t handle;
    if (nvs_open(NVS_NAMESPACE, NVS_READONLY, &handle) != ESP_OK) {
        return false;
    }
    uint8_t value = 0;
    const esp_err_t err = nvs_get_u8(handle, NVS_ENABLED, &value);
    nvs_close(handle);
    return err == ESP_OK && value == 1;
}

static int port_save_enabled(void *ctx, bool on)
{
    (void)ctx;
    nvs_handle_t handle;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &handle);
    if (err != ESP_OK) {
        return err;
    }
    err = nvs_set_u8(handle, NVS_ENABLED, on ? 1 : 0);
    if (err == ESP_OK) {
        err = nvs_commit(handle);
    }
    nvs_close(handle);
    return err;
}

static uint32_t port_now_ms(void *ctx)
{
    (void)ctx;
    return (uint32_t)(esp_timer_get_time() / 1000);
}

static int count_conn(uint16_t conn, void *arg)
{
    (void)conn;
    (*(int *)arg)++;
    return 0;
}

static int port_conn_count(void *ctx)
{
    (void)ctx;
    int count = 0;
    ble_gap_conn_foreach_handle(count_conn, &count);
    return count;
}

static void addr_from(td_ble_addr_t *out, const ble_addr_t *in)
{
    out->type = in->type;
    memcpy(out->val, in->val, sizeof out->val);
}

static void addr_to(ble_addr_t *out, const td_ble_addr_t *in)
{
    out->type = in->type;
    memcpy(out->val, in->val, sizeof out->val);
}

static bool port_link(void *ctx, uint16_t conn, td_ble_link_t *out)
{
    (void)ctx;
    struct ble_gap_conn_desc desc;
    if (ble_gap_conn_find(conn, &desc) != 0) {
        return false;
    }
    out->encrypted = desc.sec_state.encrypted;
    out->bonded = desc.sec_state.bonded;
    out->authenticated = desc.sec_state.authenticated;
    out->key_size = desc.sec_state.key_size;
    addr_from(&out->peer, &desc.peer_id_addr);
    /* Whether the bond's keys came from Secure Connections is in the store; NimBLE writes a new bond there before it
     * reports the encryption change. */
    out->secure_connections = false;
    if (desc.sec_state.bonded) {
        struct ble_store_key_sec key = { .peer_addr = desc.peer_id_addr };
        struct ble_store_value_sec value;
        if (ble_store_read_peer_sec(&key, &value) == 0) {
            out->secure_connections = value.sc;
        }
    }
    return true;
}

static size_t port_bonds(void *ctx, td_ble_addr_t out[TD_BLE_BONDS_MAX])
{
    (void)ctx;
    ble_addr_t peers[MYNEWT_VAL(BLE_STORE_MAX_BONDS)];
    int count = 0;
    if (ble_store_util_bonded_peers(peers, &count, MYNEWT_VAL(BLE_STORE_MAX_BONDS)) != 0) {
        return 0;
    }
    if (count > TD_BLE_BONDS_MAX) {
        count = TD_BLE_BONDS_MAX;
    }
    for (int i = 0; i < count; i++) {
        addr_from(&out[i], &peers[i]);
    }
    return (size_t)count;
}

static int port_bond_delete(void *ctx, const td_ble_addr_t *peer)
{
    (void)ctx;
    ble_addr_t addr;
    addr_to(&addr, peer);
    /* Also drops a link to that host. */
    return ble_gap_unpair(&addr);
}

static int port_keys_delete(void *ctx, const td_ble_addr_t *peer)
{
    (void)ctx;
    ble_addr_t addr;
    addr_to(&addr, peer);
    return ble_store_util_delete_peer(&addr);
}

static int port_accept_list(void *ctx, const td_ble_addr_t *peers, size_t count)
{
    (void)ctx;
    ble_addr_t addrs[TD_BLE_BONDS_MAX];
    for (size_t i = 0; i < count; i++) {
        addr_to(&addrs[i], &peers[i]);
    }
    const int rc = ble_gap_wl_set(addrs, (uint8_t)count);
    ESP_LOGI(TAG, "accept list: %u hosts, rc=%d", (unsigned)count, rc);
    return rc;
}

/* The controller resolves a bonded host's resolvable private address to its identity address with the IRK the host gave
 * when it bonded, and the accept list then matches that identity. NimBLE loads the stored IRKs at sync and adds a new
 * bond's IRK as it stores it; writing them again with the accept list keeps the two lists in step whatever NimBLE did.
 * Each entry is removed first, so an entry already there is replaced, never doubled. Called only while nothing
 * advertises and no connection exists, as the controller requires. */
static int port_resolving_list(void *ctx, const td_ble_addr_t *peers, size_t count)
{
    (void)ctx;
    int failed = 0;
    size_t written = 0;
    for (size_t i = 0; i < count; i++) {
        struct ble_store_key_sec key = { 0 };
        addr_to(&key.peer_addr, &peers[i]);
        struct ble_store_value_sec value;
        if (ble_store_read_peer_sec(&key, &value) != 0 || !value.irk_present) {
            continue;
        }
        ble_hs_pvcy_remove_entry(value.peer_addr.type, value.peer_addr.val);
        const int rc = ble_hs_pvcy_add_entry(value.peer_addr.val, value.peer_addr.type, value.irk);
        if (rc != 0) {
            failed = rc;
        } else {
            written++;
        }
    }
    ESP_LOGI(TAG, "resolving list: %u of %u hosts, rc=%d", (unsigned)written, (unsigned)count, failed);
    return failed;
}

/* The Device name in the scan response while pairing: complete when it fits, else cut at a character boundary. */
static void scan_response_name(struct ble_hs_adv_fields *fields)
{
    size_t len = strlen(s->name);
    const size_t room = BLE_HS_ADV_MAX_SZ - 2;
    fields->name_is_complete = len <= room;
    if (len > room) {
        len = room;
        while (len > 0 && ((unsigned char)s->name[len] & 0xC0) == 0x80) {
            len--;
        }
    }
    fields->name = (const uint8_t *)s->name;
    fields->name_len = (uint8_t)len;
}

static int port_adv_start(void *ctx, td_ble_adv_t mode, uint32_t duration_ms)
{
    (void)ctx;
    const bool pairing = mode == TD_BLE_ADV_PAIRING;
    ble_uuid16_t hid = BLE_UUID16_INIT(TD_BLE_SVC_HID);
    struct ble_hs_adv_fields fields = {
        .flags = BLE_HS_ADV_F_BREDR_UNSUP | (pairing ? BLE_HS_ADV_F_DISC_GEN : 0),
        .uuids16 = &hid,
        .num_uuids16 = 1,
        .uuids16_is_complete = 1,
        .appearance = APPEARANCE_KEYBOARD,
        .appearance_is_present = 1,
    };
    int rc = ble_gap_adv_set_fields(&fields);
    if (rc != 0) {
        return rc;
    }
    /* The name goes out only while pairing. Reconnect advertising carries none. */
    struct ble_hs_adv_fields response = { 0 };
    if (pairing) {
        scan_response_name(&response);
    }
    rc = ble_gap_adv_rsp_set_fields(&response);
    if (rc != 0) {
        return rc;
    }
    struct ble_gap_adv_params params = {
        .conn_mode = BLE_GAP_CONN_MODE_UND,
        .disc_mode = pairing ? BLE_GAP_DISC_MODE_GEN : BLE_GAP_DISC_MODE_NON,
        .itvl_min = pairing ? PAIRING_ITVL_MIN : mode == TD_BLE_ADV_FAST ? FAST_ITVL_MIN : SLOW_ITVL_MIN,
        .itvl_max = pairing ? PAIRING_ITVL_MAX : mode == TD_BLE_ADV_FAST ? FAST_ITVL_MAX : SLOW_ITVL_MAX,
        /* Outside the pairing window only bonded hosts may scan or connect. */
        .filter_policy = pairing ? BLE_HCI_ADV_FILT_NONE : BLE_HCI_ADV_FILT_BOTH,
    };
    const int32_t duration = duration_ms ? (int32_t)duration_ms : BLE_HS_FOREVER;
    rc = ble_gap_adv_start(s->own_addr_type, NULL, duration, &params, gap_event, NULL);
    ESP_LOGI(TAG, "advertising %s for %ld ms, rc=%d", pairing ? "to pair" : mode == TD_BLE_ADV_FAST ? "fast" : "slow",
             duration_ms ? (long)duration_ms : -1L, rc);
    return rc;
}

static void port_adv_stop(void *ctx)
{
    (void)ctx;
    ble_gap_adv_stop();
}

static int port_security(void *ctx, uint16_t conn)
{
    (void)ctx;
    return ble_gap_security_initiate(conn);
}

static int port_conn_params(void *ctx, uint16_t conn)
{
    (void)ctx;
    const struct ble_gap_upd_params params = {
        .itvl_min = CONN_ITVL_MIN,
        .itvl_max = CONN_ITVL_MAX,
        .latency = CONN_LATENCY,
        .supervision_timeout = CONN_TIMEOUT,
    };
    return ble_gap_update_params(conn, &params);
}

static void port_terminate(void *ctx, uint16_t conn)
{
    (void)ctx;
    ble_gap_terminate(conn, BLE_ERR_REM_USER_CONN_TERM);
}

static void port_timer(void *ctx, uint32_t delay_ms)
{
    (void)ctx;
    if (!__atomic_load_n(&s_post_ok, __ATOMIC_SEQ_CST)) {
        return;
    }
    ble_npl_callout_reset(&s->timer, ble_npl_time_ms_to_ticks32(delay_ms ? delay_ms : 1));
}

static void port_timer_cancel(void *ctx)
{
    (void)ctx;
    if (__atomic_load_n(&s_post_ok, __ATOMIC_SEQ_CST)) {
        ble_npl_callout_stop(&s->timer);
    }
}

static uint16_t input_handle(td_ble_input_t input)
{
    switch (input) {
    case TD_BLE_IN_KEYBOARD: return s->handles[TD_BLE_ATTR_KEYBOARD_IN];
    case TD_BLE_IN_CONSUMER: return s->handles[TD_BLE_ATTR_CONSUMER_IN];
    default: return s->handles[TD_BLE_ATTR_BOOT_KEYBOARD_IN];
    }
}

static int port_notify(void *ctx, uint16_t conn, td_ble_input_t input, const uint8_t *data, size_t len)
{
    (void)ctx;
    struct os_mbuf *om = ble_hs_mbuf_from_flat(data, (uint16_t)len);
    return om ? ble_gatts_notify_custom(conn, input_handle(input), om) : BLE_HS_ENOMEM;
}

static uint32_t port_random(void *ctx)
{
    (void)ctx;
    return esp_random();
}

/* The code goes to the screen only. It is never logged, since the USB log is readable by the computer at the other end
 * of the cable. */
static void port_passkey_show(void *ctx, bool show, uint32_t passkey)
{
    (void)ctx;
    const td_ble_passkey_hook_t hook = __atomic_load_n(&s_passkey_hook, __ATOMIC_SEQ_CST);
    if (hook) {
        hook(show, passkey);
    } else if (show) {
        ESP_LOGW(TAG, "no screen to show the passkey on");
    }
    ESP_LOGI(TAG, "passkey %s", show ? "on screen" : "taken down");
}

static void port_lock(void *ctx)
{
    (void)ctx;
    portENTER_CRITICAL(&s_queue_lock);
}

static void port_unlock(void *ctx)
{
    (void)ctx;
    portEXIT_CRITICAL(&s_queue_lock);
}

/* ---- GATT. ---- */

static int append(struct os_mbuf *om, const void *data, size_t len)
{
    return os_mbuf_append(om, data, (uint16_t)len) == 0 ? 0 : BLE_ATT_ERR_INSUFFICIENT_RES;
}

static int write_byte(struct ble_gatt_access_ctxt *ctxt, uint8_t *out)
{
    if (OS_MBUF_PKTLEN(ctxt->om) != 1 || os_mbuf_copydata(ctxt->om, 0, 1, out) != 0) {
        return BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
    }
    return 0;
}

static int access_locked(uint16_t conn, const td_ble_chr_t *chr, struct ble_gatt_access_ctxt *ctxt)
{
    if ((chr->flags & (TD_BLE_F_ENC | TD_BLE_F_AUTHEN)) && !td_ble_core_may_access(&s->core, conn)) {
        return BLE_ATT_ERR_INSUFFICIENT_AUTHEN;
    }
    if (ctxt->op == BLE_GATT_ACCESS_OP_READ_DSC) {
        const uint8_t reference[] = { chr->report_id, chr->report_type };
        return append(ctxt->om, reference, sizeof reference);
    }
    if (ctxt->op == BLE_GATT_ACCESS_OP_WRITE_CHR) {
        uint8_t value;
        int rc = write_byte(ctxt, &value);
        if (rc != 0) {
            return rc;
        }
        switch (chr->attr) {
        case TD_BLE_ATTR_CONTROL_POINT:
            return td_ble_core_control_point(&s->core, conn, value) ? 0 : BLE_ATT_ERR_UNLIKELY;
        case TD_BLE_ATTR_PROTOCOL_MODE:
            return td_ble_core_set_protocol(&s->core, conn, value) ? 0 : BLE_ATT_ERR_UNLIKELY;
        case TD_BLE_ATTR_LED_OUT:
        case TD_BLE_ATTR_BOOT_KEYBOARD_OUT:
            td_ble_core_set_leds(&s->core, conn, value);
            return 0;
        default:
            return BLE_ATT_ERR_WRITE_NOT_PERMITTED;
        }
    }
    uint8_t report[TD_BLE_KEYBOARD_BYTES];
    uint8_t byte;
    switch (chr->attr) {
    case TD_BLE_ATTR_REPORT_MAP: return append(ctxt->om, td_ble_report_map, td_ble_report_map_len);
    case TD_BLE_ATTR_HID_INFO: return append(ctxt->om, td_ble_hid_info, sizeof td_ble_hid_info);
    case TD_BLE_ATTR_PNP_ID: return append(ctxt->om, td_ble_pnp_id, sizeof td_ble_pnp_id);
    case TD_BLE_ATTR_BATTERY_LEVEL:
        byte = TD_BLE_BATTERY_LEVEL;
        return append(ctxt->om, &byte, 1);
    case TD_BLE_ATTR_PROTOCOL_MODE:
        byte = td_ble_core_protocol(&s->core, conn);
        return append(ctxt->om, &byte, 1);
    case TD_BLE_ATTR_LED_OUT:
    case TD_BLE_ATTR_BOOT_KEYBOARD_OUT:
        byte = td_ble_core_leds(&s->core, conn);
        return append(ctxt->om, &byte, 1);
    case TD_BLE_ATTR_KEYBOARD_IN:
        return append(ctxt->om, report, td_ble_core_current(&s->core, TD_BLE_IN_KEYBOARD, report));
    case TD_BLE_ATTR_CONSUMER_IN:
        return append(ctxt->om, report, td_ble_core_current(&s->core, TD_BLE_IN_CONSUMER, report));
    case TD_BLE_ATTR_BOOT_KEYBOARD_IN:
        return append(ctxt->om, report, td_ble_core_current(&s->core, TD_BLE_IN_BOOT_KEYBOARD, report));
    default:
        return BLE_ATT_ERR_READ_NOT_PERMITTED;
    }
}

static int gatt_access(uint16_t conn, uint16_t attr_handle, struct ble_gatt_access_ctxt *ctxt, void *arg)
{
    (void)attr_handle;
    lock();
    const int rc = access_locked(conn, (const td_ble_chr_t *)arg, ctxt);
    unlock();
    return rc;
}

/* NimBLE's service table, made once from td_ble_chrs into PSRAM: one service per run of characteristics with the same
 * service UUID, and a Report Reference descriptor on each Report characteristic. */
static esp_err_t build_services(void)
{
    if (s->services) {
        return ESP_OK;
    }
    size_t services = 0;
    for (size_t i = 0; i < td_ble_chr_count; i++) {
        services += i == 0 || td_ble_chrs[i].service != td_ble_chrs[i - 1].service;
    }
    const size_t bytes = (services + 1) * sizeof(struct ble_gatt_svc_def) +
                         (td_ble_chr_count + services) * sizeof(struct ble_gatt_chr_def) +
                         td_ble_chr_count * 2 * sizeof(struct ble_gatt_dsc_def) +
                         (services + 2 * td_ble_chr_count) * sizeof(ble_uuid16_t);
    uint8_t *memory = heap_caps_calloc(1, bytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (!memory) {
        return ESP_ERR_NO_MEM;
    }
    struct ble_gatt_svc_def *svc = (struct ble_gatt_svc_def *)memory;
    struct ble_gatt_chr_def *chr = (struct ble_gatt_chr_def *)(svc + services + 1);
    struct ble_gatt_dsc_def *dsc = (struct ble_gatt_dsc_def *)(chr + td_ble_chr_count + services);
    ble_uuid16_t *uuid = (ble_uuid16_t *)(dsc + td_ble_chr_count * 2);
    size_t n = 0;
    for (size_t i = 0; i < td_ble_chr_count; i++) {
        const td_ble_chr_t *c = &td_ble_chrs[i];
        if (i == 0 || c->service != td_ble_chrs[i - 1].service) {
            if (i != 0) {
                chr++; /* the zeroed terminator of the previous service */
            }
            *uuid = (ble_uuid16_t)BLE_UUID16_INIT(c->service);
            svc[n++] = (struct ble_gatt_svc_def){ .type = BLE_GATT_SVC_TYPE_PRIMARY, .uuid = &uuid->u,
                                                  .characteristics = chr };
            uuid++;
        }
        *uuid = (ble_uuid16_t)BLE_UUID16_INIT(c->uuid);
        *chr = (struct ble_gatt_chr_def){ .uuid = &uuid->u, .access_cb = gatt_access, .arg = (void *)c,
                                          .flags = td_ble_gatt_flags(c->flags), .val_handle = &s->handles[c->attr] };
        uuid++;
        if (c->report_id) {
            *uuid = (ble_uuid16_t)BLE_UUID16_INIT(0x2908);
            dsc[0] = (struct ble_gatt_dsc_def){
                .uuid = &uuid->u,
                .att_flags = td_ble_report_ref_flags(c->flags),
                .access_cb = gatt_access,
                .arg = (void *)c,
            };
            chr->descriptors = dsc;
            dsc += 2;
            uuid++;
        }
        chr++;
    }
    s->services = svc;
    return ESP_OK;
}

/* ---- The host. ---- */

/* A GATT layout or report map a bonded host may have cached has changed since the last boot that ran BLE: tell bonded
 * hosts through Service Changed. The PnP version moved with it (td_ble_hid.h). */
static void signal_gatt_version(void)
{
    nvs_handle_t handle;
    if (nvs_open(NVS_NAMESPACE, NVS_READWRITE, &handle) != ESP_OK) {
        return;
    }
    uint16_t stored = 0;
    if (nvs_get_u16(handle, NVS_GATT_VERSION, &stored) != ESP_OK || stored != TD_BLE_GATT_VERSION) {
        ble_svc_gatt_changed(0x0001, 0xFFFF);
        if (nvs_set_u16(handle, NVS_GATT_VERSION, TD_BLE_GATT_VERSION) == ESP_OK) {
            nvs_commit(handle);
        }
        ESP_LOGI(TAG, "GATT version %u (was %u): Service Changed raised", TD_BLE_GATT_VERSION, stored);
    }
    nvs_close(handle);
}

static void on_sync(void)
{
    if (ble_hs_util_ensure_addr(0) != 0 || ble_hs_id_infer_auto(1, &s->own_addr_type) != 0) {
        ESP_LOGE(TAG, "no usable address");
        return;
    }
    signal_gatt_version();
    lock();
    td_ble_core_synced(&s->core);
    unlock();
    ESP_LOGI(TAG, "ready as \"%s\"", s->name);
}

static void on_reset(int reason)
{
    lock();
    td_ble_core_host_reset(&s->core);
    unlock();
    ESP_LOGE(TAG, "host reset %d", reason);
}

static void on_timer(struct ble_npl_event *event)
{
    (void)event;
    lock();
    td_ble_core_timer(&s->core);
    unlock();
}

static void on_drain(struct ble_npl_event *event)
{
    (void)event;
    lock();
    td_ble_core_drain(&s->core);
    unlock();
}

static td_ble_input_t input_of(uint16_t attr_handle, bool *found)
{
    *found = true;
    if (attr_handle == s->handles[TD_BLE_ATTR_KEYBOARD_IN]) return TD_BLE_IN_KEYBOARD;
    if (attr_handle == s->handles[TD_BLE_ATTR_CONSUMER_IN]) return TD_BLE_IN_CONSUMER;
    if (attr_handle == s->handles[TD_BLE_ATTR_BOOT_KEYBOARD_IN]) return TD_BLE_IN_BOOT_KEYBOARD;
    *found = false;
    return TD_BLE_IN_COUNT;
}

static int gap_event(struct ble_gap_event *event, void *arg)
{
    (void)arg;
    int rc = 0;
    bool found;
    td_ble_input_t input;
    if (event->type == BLE_GAP_EVENT_NOTIFY_TX) {
        /* NimBLE raises this inside ble_gatts_notify_custom, with s_mutex held by the drain that sent it. The drain
         * already saw the return code, so there is nothing to do and nothing to lock. */
        return 0;
    }
    if (event->type == BLE_GAP_EVENT_PASSKEY_ACTION) {
        /* The core puts the code on screen and the answer goes to the Security Manager outside s_mutex, because a
         * failed answer raises its pairing events from inside ble_sm_inject_io. */
        const uint16_t conn = event->passkey.conn_handle;
        const td_ble_io_t action =
            event->passkey.params.action == BLE_SM_IOACT_DISP ? TD_BLE_IO_DISPLAY : TD_BLE_IO_OTHER;
        uint32_t passkey = 0;
        lock();
        const bool answer = td_ble_core_passkey_action(&s->core, conn, action, &passkey);
        unlock();
        ESP_LOGI(TAG, "pairing conn=%u action=%u: %s", conn, event->passkey.params.action,
                 answer ? "passkey shown" : "refused");
        if (answer) {
            struct ble_sm_io io = { .action = BLE_SM_IOACT_DISP, .passkey = passkey };
            const int rc = ble_sm_inject_io(conn, &io);
            if (rc != 0) {
                lock();
                td_ble_core_pairing_complete(&s->core, conn, rc);
                unlock();
            }
        }
        return 0;
    }
    lock();
    switch (event->type) {
    case BLE_GAP_EVENT_CONNECT:
        ESP_LOGI(TAG, "connect conn=%u status=%d", event->connect.conn_handle, event->connect.status);
        td_ble_core_connect(&s->core, event->connect.conn_handle, event->connect.status);
        break;
    case BLE_GAP_EVENT_DISCONNECT:
        ESP_LOGI(TAG, "disconnect conn=%u reason=0x%x", event->disconnect.conn.conn_handle, event->disconnect.reason);
        td_ble_core_disconnect(&s->core, event->disconnect.conn.conn_handle);
        break;
    case BLE_GAP_EVENT_ENC_CHANGE:
        ESP_LOGI(TAG, "encryption conn=%u status=%d", event->enc_change.conn_handle, event->enc_change.status);
        td_ble_core_enc_change(&s->core, event->enc_change.conn_handle, event->enc_change.status);
        break;
    case BLE_GAP_EVENT_PARING_COMPLETE:
        ESP_LOGI(TAG, "pairing conn=%u status=%d", event->pairing_complete.conn_handle, event->pairing_complete.status);
        td_ble_core_pairing_complete(&s->core, event->pairing_complete.conn_handle, event->pairing_complete.status);
        break;
    case BLE_GAP_EVENT_ADV_COMPLETE:
        td_ble_core_adv_complete(&s->core, event->adv_complete.reason == BLE_HS_ETIMEOUT);
        break;
    case BLE_GAP_EVENT_SUBSCRIBE:
        input = input_of(event->subscribe.attr_handle, &found);
        if (found) {
            td_ble_core_subscribe(&s->core, event->subscribe.conn_handle, input, event->subscribe.cur_notify);
        }
        break;
    case BLE_GAP_EVENT_CONN_UPDATE:
        /* The host's answer to the one request. A refusal stands; nothing is retried. */
        ESP_LOGI(TAG, "connection parameters conn=%u status=%d", event->conn_update.conn_handle,
                 event->conn_update.status);
        break;
    case BLE_GAP_EVENT_REPEAT_PAIRING:
        rc = td_ble_core_repeat_pairing(&s->core, event->repeat_pairing.conn_handle) ? BLE_GAP_REPEAT_PAIRING_RETRY
                                                                                      : BLE_GAP_REPEAT_PAIRING_IGNORE;
        break;
    default:
        break;
    }
    unlock();
    return rc;
}

static void host_task(void *arg)
{
    (void)arg;
    nimble_port_run();
    nimble_port_freertos_deinit();
}

static int port_stack_start(void *ctx)
{
    (void)ctx;
    if (build_services() != ESP_OK) {
        return ESP_ERR_NO_MEM;
    }
    esp_err_t err = nimble_port_init();
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "controller and host did not start: %s", esp_err_to_name(err));
        return err;
    }
    ble_hs_cfg.sync_cb = on_sync;
    ble_hs_cfg.reset_cb = on_reset;
    ble_hs_cfg.store_status_cb = ble_store_util_status_rr;
    /* LE Secure Connections only, with MITM protection, bonding, inside the pairing window the person opens. The Device
     * is display-only: the host pairs by typing the six-digit passkey the Device shows. A host that can't type gets
     * Just Works, whose unauthenticated keys the core refuses (td_ble_core.h). */
    ble_hs_cfg.sm_io_cap = BLE_HS_IO_DISPLAY_ONLY;
    ble_hs_cfg.sm_bonding = 1;
    ble_hs_cfg.sm_mitm = 1;
    ble_hs_cfg.sm_sc = 1;
    ble_hs_cfg.sm_sc_only = 1;
    ble_hs_cfg.sm_our_key_dist = BLE_SM_PAIR_KEY_DIST_ENC | BLE_SM_PAIR_KEY_DIST_ID;
    ble_hs_cfg.sm_their_key_dist = BLE_SM_PAIR_KEY_DIST_ENC | BLE_SM_PAIR_KEY_DIST_ID;
    ble_svc_gap_init();
    ble_svc_gatt_init();
    int rc = ble_gatts_count_cfg(s->services);
    if (rc == 0) {
        rc = ble_gatts_add_svcs(s->services);
    }
    if (rc != 0) {
        ESP_LOGE(TAG, "GATT table refused: %d", rc);
        nimble_port_deinit();
        return rc;
    }
    if (td_device_name_get(s->name) != ESP_OK) {
        strcpy(s->name, "Open TouchDeck");
    }
    /* GAP is readable before authentication; only pairing advertisements carry the chosen name. */
    ble_svc_gap_device_name_set("Open TouchDeck");
    ble_svc_gap_device_appearance_set(APPEARANCE_KEYBOARD);
    ble_store_config_init();
    ble_npl_callout_init(&s->timer, nimble_port_get_dflt_eventq(), on_timer, NULL);
    ble_npl_event_init(&s->drain, on_drain, NULL);
    __atomic_store_n(&s_post_ok, true, __ATOMIC_SEQ_CST);
    nimble_port_freertos_init(host_task);
    ESP_LOGI(TAG, "started");
    return 0;
}

static void port_stack_stop(void *ctx)
{
    (void)ctx;
    __atomic_store_n(&s_post_ok, false, __ATOMIC_SEQ_CST);
    while (__atomic_load_n(&s_posting, __ATOMIC_SEQ_CST) != 0) {
        vTaskDelay(1);
    }
    ble_npl_callout_stop(&s->timer);
    /* nimble_port_stop waits for the host task, which may be waiting for s_mutex in an event. Let it finish; the core
     * finds the setting off by now. */
    unlock();
    const int rc = nimble_port_stop();
    if (rc == 0) {
        ble_npl_callout_deinit(&s->timer);
        nimble_port_deinit();
    } else {
        ESP_LOGE(TAG, "host did not stop: %d", rc);
    }
    lock();
    ESP_LOGI(TAG, "stopped");
}

/* ---- The key sink. ---- */

static void sink_report(void *ctx, const key_report_t *report)
{
    (void)ctx;
    if (!s || !td_ble_core_report(&s->core, report)) {
        return;
    }
    __atomic_add_fetch(&s_posting, 1, __ATOMIC_SEQ_CST);
    if (__atomic_load_n(&s_post_ok, __ATOMIC_SEQ_CST)) {
        ble_npl_eventq_put(nimble_port_get_dflt_eventq(), &s->drain);
    }
    __atomic_sub_fetch(&s_posting, 1, __ATOMIC_SEQ_CST);
}

key_sink_t td_ble_key_sink(void)
{
    return (key_sink_t){ .report = sink_report };
}

/* ---- The serialized service and the entry points. ---- */

td_ble_result_t td_ble_set_enabled(bool on)
{
    lock();
    const td_ble_result_t result = td_ble_core_set_enabled(&s->core, on);
    unlock();
    ESP_LOGI(TAG, "turned %s: %d", on ? "on" : "off", result);
    return result;
}

td_ble_result_t td_ble_pair(bool open)
{
    lock();
    const td_ble_result_t result = td_ble_core_pair(&s->core, open);
    unlock();
    return result;
}

td_ble_result_t td_ble_forget(const td_ble_addr_t *peer, size_t *forgotten)
{
    lock();
    const td_ble_result_t result = td_ble_core_forget(&s->core, peer, forgotten);
    unlock();
    return result;
}

void td_ble_status(td_ble_status_t *out)
{
    lock();
    td_ble_core_status(&s->core, out);
    unlock();
}

const char *td_ble_state_name(void)
{
    if (!s) {
        return "off";
    }
    td_ble_status_t status;
    td_ble_status(&status);
    if (status.start_failed) return "failed";
    if (!status.running) return "off";
    return status.synced ? "on" : "starting";
}

void td_ble_set_passkey_hook(td_ble_passkey_hook_t hook)
{
    __atomic_store_n(&s_passkey_hook, hook, __ATOMIC_SEQ_CST);
}

static const td_link_extension_t s_extension = { .prefix = "ble.", .handle = td_ble_message, .caps = td_ble_caps };

esp_err_t td_ble_init(void)
{
    if (s) {
        return ESP_ERR_INVALID_STATE;
    }
    ble_t *ble = heap_caps_calloc(1, sizeof *ble, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (!ble) {
        return ESP_ERR_NO_MEM;
    }
    ble->mutex = xSemaphoreCreateMutex();
    if (!ble->mutex) {
        heap_caps_free(ble);
        return ESP_ERR_NO_MEM;
    }
    ble->port = (td_ble_port_t){
        .load_enabled = port_load_enabled,
        .save_enabled = port_save_enabled,
        .stack_start = port_stack_start,
        .stack_stop = port_stack_stop,
        .now_ms = port_now_ms,
        .conn_count = port_conn_count,
        .link = port_link,
        .bonds = port_bonds,
        .bond_delete = port_bond_delete,
        .keys_delete = port_keys_delete,
        .accept_list = port_accept_list,
        .resolving_list = port_resolving_list,
        .adv_start = port_adv_start,
        .adv_stop = port_adv_stop,
        .security = port_security,
        .conn_params = port_conn_params,
        .terminate = port_terminate,
        .timer = port_timer,
        .timer_cancel = port_timer_cancel,
        .notify = port_notify,
        .random = port_random,
        .passkey_show = port_passkey_show,
        .lock = port_lock,
        .unlock = port_unlock,
    };
    td_ble_core_init(&ble->core, &ble->port);
    s = ble;
    return td_link_extend(&s_extension);
}

esp_err_t td_ble_boot(void)
{
    if (!s) {
        return ESP_ERR_INVALID_STATE;
    }
    /* The NVS partition the setting lives in; td_device opened it already, so this is a no-op. */
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) {
        return err;
    }
    lock();
    td_ble_core_boot(&s->core);
    td_ble_status_t status;
    td_ble_core_status(&s->core, &status);
    unlock();
    ESP_LOGI(TAG, "%s", status.start_failed ? "failed to start" : status.running ? "on" : "off, as stored");
    return status.start_failed ? ESP_FAIL : ESP_OK;
}
