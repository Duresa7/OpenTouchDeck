#include "td_netlink_manage.h"
#include <string.h>

static void refuse(cJSON *reply, const char *reason, const char *error)
{
    cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
    cJSON_AddStringToObject(reply, "reason", reason);
    cJSON_AddStringToObject(reply, "error", error);
}

void td_netlink_app_id(const uint8_t companion_id[TD_NETLINK_COMPANION_ID_LEN], char out[TD_NETLINK_APP_ID_LEN + 1])
{
    static const char digits[] = "0123456789abcdef";
    for (size_t i = 0; i < TD_NETLINK_COMPANION_ID_LEN; i++) {
        out[2 * i] = digits[companion_id[i] >> 4];
        out[2 * i + 1] = digits[companion_id[i] & 15];
    }
    out[TD_NETLINK_APP_ID_LEN] = '\0';
}

static int nibble(char c)
{
    if (c >= '0' && c <= '9') {
        return c - '0';
    }
    return c >= 'a' && c <= 'f' ? c - 'a' + 10 : -1;
}

bool td_netlink_parse_app_id(const char *text, uint8_t out[TD_NETLINK_COMPANION_ID_LEN])
{
    if (!text || strlen(text) != TD_NETLINK_APP_ID_LEN) {
        return false;
    }
    for (size_t i = 0; i < TD_NETLINK_COMPANION_ID_LEN; i++) {
        const int hi = nibble(text[2 * i]);
        const int lo = nibble(text[2 * i + 1]);
        if (hi < 0 || lo < 0) {
            return false;
        }
        out[i] = (uint8_t)(hi << 4 | lo);
    }
    return true;
}

void td_netlink_deck_fields(const td_netlink_deck_t *deck, cJSON *reply)
{
    cJSON_AddNumberToObject(reply, "paired", deck->pairings.count);
    cJSON_AddBoolToObject(reply, "pairing", td_netlink_window_left(deck) > 0);
    cJSON_AddNumberToObject(reply, "pair_failures", deck->pair_failures);
    cJSON_AddNumberToObject(reply, "locked_ms", td_netlink_lock_left(deck));
}

static void state_fields(const td_netlink_deck_t *deck, cJSON *reply, const td_netlink_manage_env_t *env)
{
    env->base_fields(env->ctx, reply);
    td_netlink_deck_fields(deck, reply);
}

static void pair(td_netlink_deck_t *deck, cJSON *reply, const td_netlink_manage_env_t *env)
{
    switch (td_netlink_window_open(deck)) {
    case TD_NETLINK_REASON_NONE:
        state_fields(deck, reply, env);
        break;
    case TD_NETLINK_FULL:
        refuse(reply, "full", TD_NETLINK_FULL_ERROR);
        break;
    default:
        refuse(reply, "locked", "Pairing is locked after failed codes; try again later");
        break;
    }
}

static void list(const td_netlink_deck_t *deck, cJSON *reply, const td_netlink_manage_env_t *env)
{
    state_fields(deck, reply, env);
    cJSON_AddNumberToObject(reply, "free", TD_NETLINK_PAIRINGS_MAX - deck->pairings.count);
    cJSON *pairings = cJSON_AddArrayToObject(reply, "pairings");
    for (uint8_t i = 0; pairings && i < deck->pairings.count; i++) {
        const td_netlink_pairing_t *p = &deck->pairings.entries[i];
        char app_id[TD_NETLINK_APP_ID_LEN + 1];
        char name[TD_NETLINK_NAME_MAX + 1];
        const size_t name_len = p->name_len > TD_NETLINK_NAME_MAX ? TD_NETLINK_NAME_MAX : p->name_len;
        memcpy(name, p->name, name_len);
        name[name_len] = '\0';
        td_netlink_app_id(p->companion_id, app_id);
        cJSON *entry = cJSON_CreateObject();
        if (!entry) {
            break;
        }
        cJSON_AddStringToObject(entry, "app_id", app_id);
        cJSON_AddStringToObject(entry, "app_name", name);
        cJSON_AddItemToArray(pairings, entry);
    }
}

/* With app_id, one Pairing; with all:true, every one. Anything else is refused before a session ends, so a request
 * that lost its app_id can't wipe the Device. */
static void forget(td_netlink_deck_t *deck, const cJSON *request, cJSON *reply, const td_netlink_manage_env_t *env)
{
    const cJSON *app_id = cJSON_GetObjectItemCaseSensitive(request, "app_id");
    const cJSON *all = cJSON_GetObjectItemCaseSensitive(request, "all");
    uint8_t id[TD_NETLINK_COMPANION_ID_LEN];
    uint8_t removed = 0;
    bool stored;
    if (app_id) {
        if (all || !td_netlink_parse_app_id(cJSON_GetStringValue(app_id), id)) {
            refuse(reply, "args", "Invalid app_id");
            return;
        }
        env->end_sessions(env->ctx, id);
        stored = td_netlink_forget_one(deck, id, &removed);
    } else if (cJSON_IsTrue(all)) {
        env->end_sessions(env->ctx, NULL);
        removed = deck->pairings.count;
        stored = td_netlink_forget(deck);
    } else {
        refuse(reply, "args", "Name an app_id, or send all:true");
        return;
    }
    if (!stored) {
        refuse(reply, "storage", "The pairings could not be erased");
        return;
    }
    state_fields(deck, reply, env);
    cJSON_AddNumberToObject(reply, "removed", removed);
}

void td_netlink_manage(td_netlink_deck_t *deck, const cJSON *request, cJSON *reply,
                       const td_netlink_manage_env_t *env)
{
    const char *m = cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(request, "m"));
    if (m && !strcmp(m, "net.pair")) {
        pair(deck, reply, env);
    } else if (m && !strcmp(m, "net.list")) {
        list(deck, reply, env);
    } else if (m && !strcmp(m, "net.forget")) {
        forget(deck, request, reply, env);
    } else {
        refuse(reply, "args", "Not a Pairing message");
    }
}
