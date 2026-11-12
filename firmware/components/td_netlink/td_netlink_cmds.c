#include "td_netlink_service.h"
#include "td_netlink_priv.h"
#include "td_netlink_manage.h"
#include "td_link.h"
#include "td_console.h"
#include <string.h>

#define USAGE "usage: net state | list | pair | forget <app_id> | forget all"

/* ---- Link messages ---- */

static void net_state(const cJSON *request, cJSON *reply)
{
    (void)request;
    td_netlink_state_fields(reply);
}

/* net.pair, net.list and net.forget, all USB-only: the link refuses them on a network carrier. */
static void net_manage(const cJSON *request, cJSON *reply)
{
    if (!td_netlink_manage_request(request, reply)) {
        cJSON_ReplaceItemInObjectCaseSensitive(reply, "ok", cJSON_CreateBool(false));
        cJSON_AddStringToObject(reply, "reason", "storage");
        cJSON_AddStringToObject(reply, "error", "The network service is not running");
    }
}

static const td_link_message_t s_messages[] = {
    { "net.state", net_state, false },
    { "net.pair", net_manage, true },
    { "net.list", net_manage, true },
    { "net.forget", net_manage, true },
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

/* The console's `net` takes the link messages' words: `net forget <app_id>` is net.forget with that app_id. */
static cJSON *console_request(int argc, char **argv, const char **op)
{
    cJSON *request = cJSON_CreateObject();
    if (!request) {
        return NULL;
    }
    *op = argc >= 2 ? argv[1] : "";
    const char *m = NULL;
    if (argc == 2 && !strcmp(*op, "pair")) {
        m = "net.pair";
    } else if (argc == 2 && !strcmp(*op, "list")) {
        m = "net.list";
    } else if (argc == 3 && !strcmp(*op, "forget")) {
        m = "net.forget";
        if (!strcmp(argv[2], "all")) {
            cJSON_AddBoolToObject(request, "all", true);
        } else {
            cJSON_AddStringToObject(request, "app_id", argv[2]);
        }
    }
    if (!m) {
        cJSON_Delete(request);
        return NULL;
    }
    cJSON_AddStringToObject(request, "m", m);
    return request;
}

static int cmd_net(int argc, char **argv)
{
    const char *op = "";
    cJSON *request = NULL;
    const bool state = argc == 2 && !strcmp(argv[1], "state");
    if (!state && !(request = console_request(argc, argv, &op))) {
        char line[TD_REPLY_MAX];
        td_reply_error(line, sizeof line, "net", USAGE);
        td_console_write_line(line);
        return 1;
    }
    cJSON *reply = cJSON_CreateObject();
    if (!reply) {
        cJSON_Delete(request);
        return 1;
    }
    cJSON_AddStringToObject(reply, "cmd", "net");
    cJSON_AddBoolToObject(reply, "ok", true);
    cJSON_AddStringToObject(reply, "op", state ? "state" : op);
    if (state) {
        td_netlink_state_fields(reply);
    } else {
        net_manage(request, reply);
    }
    const bool ok = cJSON_IsTrue(cJSON_GetObjectItemCaseSensitive(reply, "ok"));
    /* Four names can outgrow one console line, so each Pairing gets its own `app` line before the reply. */
    cJSON *pairings = cJSON_DetachItemFromObjectCaseSensitive(reply, "pairings");
    const cJSON *entry;
    cJSON_ArrayForEach(entry, pairings)
    {
        cJSON *line = cJSON_Duplicate(entry, true);
        if (line) {
            cJSON_AddItemToObject(line, "cmd", cJSON_CreateString("net"));
            cJSON_AddItemToObject(line, "op", cJSON_CreateString("app"));
            write_reply(line);
            cJSON_Delete(line);
        }
    }
    cJSON_Delete(pairings);
    write_reply(reply);
    cJSON_Delete(reply);
    cJSON_Delete(request);
    return ok ? 0 : 1;
}

const esp_console_cmd_t *td_netlink_console_commands(size_t *count)
{
    static const esp_console_cmd_t commands[] = {
        { .command = "net", .help = "net state | list | pair | forget <app_id> | forget all: the network app link and "
                                    "its Pairings",
          .hint = "state", .func = cmd_net },
    };
    /* The link messages go with the commands, before the console and the link take requests. */
    static bool added;
    if (!added) {
        added = td_link_messages_add(s_messages, sizeof s_messages / sizeof s_messages[0]) == ESP_OK;
    }
    *count = sizeof commands / sizeof commands[0];
    return commands;
}
