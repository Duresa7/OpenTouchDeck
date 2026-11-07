#pragma once

/* The fields of ESP-IDF's console command that td_wifi_msgs.c fills. */

typedef int (*esp_console_cmd_func_t)(int argc, char **argv);

typedef struct {
    const char *command;
    const char *help;
    const char *hint;
    esp_console_cmd_func_t func;
    void *argtable;
} esp_console_cmd_t;
