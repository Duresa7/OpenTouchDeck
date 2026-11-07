#include "td_link.h"
#include <string.h>

static int cmd_link(int argc, char **argv)
{
    char buf[TD_REPLY_MAX];
    if (argc != 2 || strcmp(argv[1], "state") != 0) {
        td_reply_error(buf, sizeof buf, "link", "usage: link state");
        td_console_write_line(buf);
        return 1;
    }
    td_link_state_t state = td_link_state();
    td_reply_t reply;
    td_reply_begin(&reply, buf, sizeof buf, "link", true);
    td_reply_bool(&reply, "online", state.online);
    td_reply_int(&reply, "frames_rx", state.frames_rx);
    td_reply_int(&reply, "frames_tx", state.frames_tx);
    td_reply_int(&reply, "crc_errors", state.crc_errors);
    td_reply_int(&reply, "dropped", state.dropped);
    td_reply_int(&reply, "last_rx_ms", state.last_rx_ms);
    td_reply_str(&reply, "carrier", state.carrier);
    td_reply_end(&reply);
    td_console_write_line(buf);
    return 0;
}

const esp_console_cmd_t *td_link_console_commands(size_t *count)
{
    static const esp_console_cmd_t commands[] = {
        { .command = "link", .help = "report the app link state", .func = cmd_link },
    };
    *count = sizeof commands / sizeof commands[0];
    return commands;
}
