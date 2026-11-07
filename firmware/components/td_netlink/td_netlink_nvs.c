#include "td_netlink_priv.h"
#include <string.h>
#include "nvs.h"

#define NAMESPACE "td_netlink"
#define KEY_PAIRINGS "pairings"
#define KEY_FAILURES "pair_fail"
#define KEY_LOCK "lock_ms"

/* The blob: a format byte, the count, then per Pairing the companion_id, name_len, the name padded to 64 bytes, and
 * the LTK. */
#define FORMAT 1
#define ENTRY (TD_NETLINK_COMPANION_ID_LEN + 1 + TD_NETLINK_NAME_MAX + TD_NETLINK_KEY_LEN)
#define BLOB_MAX (2 + TD_NETLINK_PAIRINGS_MAX * ENTRY)

static void wipe(void *p, size_t len)
{
    td_netlink_wipe(p, len);
}

bool td_netlink_nvs_load(td_netlink_pairings_t *pairings, uint32_t *pair_failures, uint32_t *lock_left_ms)
{
    memset(pairings, 0, sizeof *pairings);
    *pair_failures = 0;
    *lock_left_ms = 0;
    nvs_handle_t nvs;
    esp_err_t err = nvs_open(NAMESPACE, NVS_READONLY, &nvs);
    if (err == ESP_ERR_NVS_NOT_FOUND) {
        return true;
    }
    if (err != ESP_OK) {
        return false;
    }
    bool ok = true;
    uint8_t blob[BLOB_MAX];
    size_t len = sizeof blob;
    err = nvs_get_blob(nvs, KEY_PAIRINGS, blob, &len);
    if (err == ESP_OK) {
        const uint8_t count = len >= 2 ? blob[1] : 0;
        if (len < 2 || blob[0] != FORMAT || count > TD_NETLINK_PAIRINGS_MAX || len != 2 + (size_t)count * ENTRY) {
            ok = false;
        } else {
            for (uint8_t i = 0; i < count; i++) {
                const uint8_t *e = blob + 2 + (size_t)i * ENTRY;
                td_netlink_pairing_t *p = &pairings->entries[i];
                memcpy(p->companion_id, e, TD_NETLINK_COMPANION_ID_LEN);
                p->name_len = e[TD_NETLINK_COMPANION_ID_LEN];
                memcpy(p->name, e + TD_NETLINK_COMPANION_ID_LEN + 1, TD_NETLINK_NAME_MAX);
                memcpy(p->ltk, e + TD_NETLINK_COMPANION_ID_LEN + 1 + TD_NETLINK_NAME_MAX, TD_NETLINK_KEY_LEN);
                if (!td_netlink_name_valid((const uint8_t *)p->name, p->name_len)) {
                    ok = false;
                }
            }
            pairings->count = ok ? count : 0;
        }
    } else if (err != ESP_ERR_NVS_NOT_FOUND) {
        ok = false;
    }
    wipe(blob, sizeof blob);
    if (!ok) {
        wipe(pairings, sizeof *pairings);
    }
    /* A counter that can't be read counts as the lockout threshold, so a broken store can't reset it. */
    err = nvs_get_u32(nvs, KEY_FAILURES, pair_failures);
    if (err != ESP_OK && err != ESP_ERR_NVS_NOT_FOUND) {
        *pair_failures = TD_NETLINK_LOCK_AFTER;
        ok = false;
    }
    err = nvs_get_u32(nvs, KEY_LOCK, lock_left_ms);
    if (err != ESP_OK && err != ESP_ERR_NVS_NOT_FOUND) {
        *lock_left_ms = TD_NETLINK_LOCK_FIRST_MS;
        ok = false;
    }
    if (*lock_left_ms > TD_NETLINK_LOCK_CAP_MS) {
        *lock_left_ms = TD_NETLINK_LOCK_CAP_MS;
    }
    nvs_close(nvs);
    return ok;
}

bool td_netlink_nvs_save_pairings(const td_netlink_pairings_t *pairings)
{
    if (pairings->count > TD_NETLINK_PAIRINGS_MAX) {
        return false;
    }
    nvs_handle_t nvs;
    if (nvs_open(NAMESPACE, NVS_READWRITE, &nvs) != ESP_OK) {
        return false;
    }
    esp_err_t err;
    if (pairings->count == 0) {
        err = nvs_erase_key(nvs, KEY_PAIRINGS);
        if (err == ESP_ERR_NVS_NOT_FOUND) {
            err = ESP_OK;
        }
    } else {
        uint8_t blob[BLOB_MAX] = { FORMAT, pairings->count };
        for (uint8_t i = 0; i < pairings->count; i++) {
            uint8_t *e = blob + 2 + (size_t)i * ENTRY;
            const td_netlink_pairing_t *p = &pairings->entries[i];
            memcpy(e, p->companion_id, TD_NETLINK_COMPANION_ID_LEN);
            e[TD_NETLINK_COMPANION_ID_LEN] = p->name_len;
            memcpy(e + TD_NETLINK_COMPANION_ID_LEN + 1, p->name, TD_NETLINK_NAME_MAX);
            memcpy(e + TD_NETLINK_COMPANION_ID_LEN + 1 + TD_NETLINK_NAME_MAX, p->ltk, TD_NETLINK_KEY_LEN);
        }
        err = nvs_set_blob(nvs, KEY_PAIRINGS, blob, 2 + (size_t)pairings->count * ENTRY);
        wipe(blob, sizeof blob);
    }
    if (err == ESP_OK) {
        err = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return err == ESP_OK;
}

bool td_netlink_nvs_save_failures(uint32_t pair_failures, uint32_t lock_left_ms)
{
    nvs_handle_t nvs;
    if (nvs_open(NAMESPACE, NVS_READWRITE, &nvs) != ESP_OK) {
        return false;
    }
    esp_err_t err = nvs_set_u32(nvs, KEY_FAILURES, pair_failures);
    if (err == ESP_OK) {
        err = nvs_set_u32(nvs, KEY_LOCK, lock_left_ms);
    }
    if (err == ESP_OK) {
        err = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return err == ESP_OK;
}
