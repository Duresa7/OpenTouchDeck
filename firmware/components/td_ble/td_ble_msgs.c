/*
 * The `ble.*` link messages. td_link sends only USB requests here (td_link_extend), so the network Carriers net-v2
 * describes can never turn Bluetooth on, open a pairing window or forget a host.
 */
#include "td_ble_service.h"
#include <stdio.h>
#include <string.h>

void td_ble_addr_format(const td_ble_addr_t *addr, char out[18])
{
    snprintf(out, 18, "%02x:%02x:%02x:%02x:%02x:%02x", addr->val[5], addr->val[4], addr->val[3], addr->val[2],
             addr->val[1], addr->val[0]);
}

static int hex_digit(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

bool td_ble_addr_parse(const char *text, td_ble_addr_t *out)
{
    if (!text || strlen(text) != 17) {
        return false;
    }
    for (int i = 0; i < 6; i++) {
        const char *p = text + 3 * i;
        const int hi = hex_digit(p[0]);
        const int lo = hex_digit(p[1]);
        if (hi < 0 || lo < 0 || (i < 5 && p[2] != ':')) {
            return false;
        }
        out->val[5 - i] = (uint8_t)(hi << 4 | lo);
    }
    out->type = 0;
    return true;
}

void td_ble_caps(cJSON *caps)
{
    cJSON_AddNumberToObject(caps, "ble_hid", TD_BLE_BONDS_MAX);
}

static const char *state_name(const td_ble_status_t *status)
{
    if (status->start_failed) return "failed";
    if (!status->running) return "off";
    return status->synced ? "on" : "starting";
}

static const char *advertising_name(const td_ble_status_t *status)
{
    if (!status->advertising) return "off";
    switch (status->adv_mode) {
    case TD_BLE_ADV_PAIRING: return "pairing";
    case TD_BLE_ADV_FAST: return "fast";
    default: return "slow";
    }
}

static void add_status(cJSON *reply, const td_ble_status_t *status)
{
    cJSON_AddBoolToObject(reply, "enabled", status->enabled);
    cJSON_AddStringToObject(reply, "state", state_name(status));
    cJSON_AddBoolToObject(reply, "pairing", status->pairing);
    cJSON_AddNumberToObject(reply, "pairing_ms", status->pairing_ms);
    cJSON_AddStringToObject(reply, "advertising", advertising_name(status));
    cJSON *bonds = cJSON_AddArrayToObject(reply, "bonds");
    for (size_t i = 0; bonds && i < status->bond_count; i++) {
        cJSON *bond = cJSON_CreateObject();
        if (!bond) {
            break;
        }
        char address[18];
        td_ble_addr_format(&status->bonds[i], address);
        cJSON_AddStringToObject(bond, "address", address);
        cJSON_AddBoolToObject(bond, "random", status->bonds[i].type != 0);
        cJSON_AddBoolToObject(bond, "connected", status->bond_connected[i]);
        cJSON_AddItemToArray(bonds, bond);
    }
    cJSON_AddNumberToObject(reply, "bonds_max", TD_BLE_BONDS_MAX);
    cJSON_AddNumberToObject(reply, "battery", TD_BLE_BATTERY_LEVEL);
}

static void fail(cJSON *reply, const char *reason, const char *error)
{
    cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
    cJSON_DeleteItemFromObjectCaseSensitive(reply, "error");
    cJSON_AddStringToObject(reply, "reason", reason);
    cJSON_AddStringToObject(reply, "error", error);
}

static void fail_result(cJSON *reply, td_ble_result_t result)
{
    switch (result) {
    case TD_BLE_OK: break;
    case TD_BLE_ERR_OFF: fail(reply, "off", "Bluetooth is off"); break;
    case TD_BLE_ERR_STARTING: fail(reply, "starting", "Bluetooth is starting, try again"); break;
    case TD_BLE_ERR_FULL: fail(reply, "full", "Too many hosts, forget one first"); break;
    case TD_BLE_ERR_STORAGE: fail(reply, "storage", "The setting could not be saved"); break;
    case TD_BLE_ERR_RADIO: fail(reply, "radio", "Bluetooth did not start"); break;
    }
}

/* True when the request has `m`, no other key outside `allowed`, and no key twice. */
static bool only(const cJSON *request, const char *allowed)
{
    const cJSON *a;
    cJSON_ArrayForEach(a, request) {
        if (strcmp(a->string, "m") != 0 && (!allowed || strcmp(a->string, allowed) != 0)) {
            return false;
        }
        for (const cJSON *b = a->next; b; b = b->next) {
            if (!strcmp(a->string, b->string)) {
                return false;
            }
        }
    }
    return true;
}

void td_ble_message(const cJSON *request, cJSON *reply)
{
    const char *name = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "m"));
    td_ble_result_t result = TD_BLE_OK;
    if (!name) {
        fail(reply, "args", "Invalid request");
        return;
    }
    if (!strcmp(name, "ble.state")) {
        if (!only(request, NULL)) {
            fail(reply, "args", "Invalid request fields");
            return;
        }
    } else if (!strcmp(name, "ble.set")) {
        const cJSON *enabled = cJSON_GetObjectItemCaseSensitive(request, "enabled");
        if (!only(request, "enabled") || !cJSON_IsBool(enabled)) {
            fail(reply, "args", "enabled must be true or false");
            return;
        }
        result = td_ble_set_enabled(cJSON_IsTrue(enabled));
    } else if (!strcmp(name, "ble.pair")) {
        const cJSON *open = cJSON_GetObjectItemCaseSensitive(request, "open");
        if (!only(request, "open") || (open && !cJSON_IsBool(open))) {
            fail(reply, "args", "open must be true or false");
            return;
        }
        result = td_ble_pair(!open || cJSON_IsTrue(open));
    } else if (!strcmp(name, "ble.forget")) {
        const cJSON *address = cJSON_GetObjectItemCaseSensitive(request, "address");
        td_ble_addr_t wanted;
        if (!only(request, "address") ||
            (address && !cJSON_IsNull(address) && !td_ble_addr_parse(cJSON_GetStringValue(address), &wanted))) {
            fail(reply, "args", "address must be aa:bb:cc:dd:ee:ff or null");
            return;
        }
        size_t forgotten = 0;
        if (!address || cJSON_IsNull(address)) {
            result = td_ble_forget(NULL, &forgotten);
        } else {
            /* Match the six bytes against the bonds, so the bond's own address type is the one deleted. */
            td_ble_status_t status;
            td_ble_status(&status);
            if (!status.enabled || !status.running) {
                result = TD_BLE_ERR_OFF;
            } else if (!status.synced) {
                result = TD_BLE_ERR_STARTING;
            }
            for (size_t i = 0; i < status.bond_count && result == TD_BLE_OK; i++) {
                if (!memcmp(status.bonds[i].val, wanted.val, sizeof wanted.val)) {
                    size_t one = 0;
                    result = td_ble_forget(&status.bonds[i], &one);
                    forgotten += one;
                }
            }
        }
        cJSON_AddNumberToObject(reply, "forgotten", (double)forgotten);
    } else {
        fail(reply, "unsupported", "Unsupported request");
        return;
    }
    fail_result(reply, result);
    td_ble_status_t status;
    td_ble_status(&status);
    add_status(reply, &status);
}
