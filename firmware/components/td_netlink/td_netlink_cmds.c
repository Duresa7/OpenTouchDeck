#include "td_netlink_service.h"
#include "td_netlink_priv.h"
#include "td_link.h"
#include "td_console.h"
#include <string.h>

#define USAGE "usage: net state | pair | forget all"

typedef struct {
    const char *reason;
    const char *error;
} refusal_t;

static refusal_t refusal(td_netlink_cmd_result_t result)
{
    switch (result) {
    case TD_NETLINK_CMD_OK:
        return (refusal_t){ NULL, NULL };
    case TD_NETLINK_CMD_LOCKED:
        return (refusal_t){ "locked", "Pairing is locked after failed codes; try again later" };
    case TD_NETLINK_CMD_FULL:
        return (refusal_t){ "full", "Four apps are paired. Forget one first." };
    case TD_NETLINK_CMD_STORAGE:
        return (refusal_t){ "storage", "The pairings could not be erased" };
    default:
        return (refusal_t){ "storage", "The network service is not running" };
    }
}

/* ---- Link messages ---- */

static void net_state(const cJSON *request, cJSON *reply)
{
    (void)request;
    td_netlink_state_fields(reply);
}

static void answer(cJSON *reply, td_netlink_cmd_result_t result)
{
    const refusal_t r = refusal(result);
    if (r.reason) {
        cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
        cJSON_AddStringToObject(reply, "reason", r.reason);
        cJSON_AddStringToObject(reply, "error", r.error);
    }
    td_netlink_state_fields(reply);
}

static void net_pair(const cJSON *request, cJSON *reply)
{
    (void)request;
    answer(reply, td_netlink_pair());
}

static void refuse_args(cJSON *reply, const char *error)
{
    cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
    cJSON_AddStringToObject(reply, "reason", "args");
    cJSON_AddStringToObject(reply, "error", error);
    td_netlink_state_fields(reply);
}

/* Only wipe-all, `{"m":"net.forget","all":true}`, for now. A bare net.forget is refused, so a request that lost its
 * app_id can't wipe the Device. */
static void net_forget(const cJSON *request, cJSON *reply)
{
    const cJSON *all = cJSON_GetObjectItemCaseSensitive(request, "all");
    const cJSON *app_id = cJSON_GetObjectItemCaseSensitive(request, "app_id");
    if (app_id) {
        refuse_args(reply, all ? "Send app_id or all, not both" : "Forgetting one app is not supported yet");
        return;
    }
    if (!cJSON_IsTrue(all)) {
        refuse_args(reply, "Send all:true to forget every app");
        return;
    }
    uint8_t removed = 0;
    const td_netlink_cmd_result_t result = td_netlink_forget_all(&removed);
    answer(reply, result);
    if (result == TD_NETLINK_CMD_OK) {
        cJSON_AddNumberToObject(reply, "removed", removed);
    }
}

static const td_link_message_t s_messages[] = {
    { "net.state", net_state, false },
    { "net.pair", net_pair, true },
    { "net.forget", net_forget, true },
};

/* ---- Console ---- */

static void write_reply(cJSON *reply)
{
    char line[TD_REPLY_MAX] = "@td ";
    if (!cJSON_PrintPreallocated(reply, line + 4, sizeof line - 4, false)) {
        td_reply_error(line, sizeof line, "net", "reply too long");
    }
    td_console_write_line(line);
}

static int cmd_net(int argc, char **argv)
{
    const char *op = argc == 2 ? argv[1] : "";
    td_netlink_cmd_result_t result = TD_NETLINK_CMD_OK;
    if (strcmp(op, "pair") == 0) {
        result = td_netlink_pair();
    } else if (argc == 3 && strcmp(argv[1], "forget") == 0 && strcmp(argv[2], "all") == 0) {
        op = "forget";
        result = td_netlink_forget_all(NULL);
    } else if (strcmp(op, "state") != 0) {
        char line[TD_REPLY_MAX];
        td_reply_error(line, sizeof line, "net", USAGE);
        td_console_write_line(line);
        return 1;
    }
    cJSON *reply = cJSON_CreateObject();
    if (!reply) {
        return 1;
    }
    const refusal_t r = refusal(result);
    cJSON_AddStringToObject(reply, "cmd", "net");
    cJSON_AddBoolToObject(reply, "ok", r.reason == NULL);
    cJSON_AddStringToObject(reply, "op", op);
    if (r.reason) {
        cJSON_AddStringToObject(reply, "reason", r.reason);
        cJSON_AddStringToObject(reply, "error", r.error);
    }
    td_netlink_state_fields(reply);
    write_reply(reply);
    cJSON_Delete(reply);
    return r.reason ? 1 : 0;
}

const esp_console_cmd_t *td_netlink_console_commands(size_t *count)
{
    static const esp_console_cmd_t commands[] = {
        { .command = "net", .help = "net state | pair | forget all: the network app link and its Pairings", .hint = "state",
          .func = cmd_net },
    };
    /* The link messages go with the commands, before the console and the link take requests. */
    static bool added;
    if (!added) {
        added = td_link_messages_add(s_messages, sizeof s_messages / sizeof s_messages[0]) == ESP_OK;
    }
    *count = sizeof commands / sizeof commands[0];
    return commands;
}
