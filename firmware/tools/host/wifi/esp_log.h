#pragma once

/* Every log line goes to wifi_test.c, which keeps them all so a test can look for a secret in them. */

void host_log(const char *level, const char *tag, const char *format, ...);

typedef enum { ESP_LOG_NONE, ESP_LOG_ERROR, ESP_LOG_WARN, ESP_LOG_INFO, ESP_LOG_DEBUG, ESP_LOG_VERBOSE } esp_log_level_t;

/* wifi_test.c records every level set, so a test can check which tags were turned down. */
void esp_log_level_set(const char *tag, esp_log_level_t level);

#define ESP_LOGE(tag, ...) host_log("E", tag, __VA_ARGS__)
#define ESP_LOGW(tag, ...) host_log("W", tag, __VA_ARGS__)
#define ESP_LOGI(tag, ...) host_log("I", tag, __VA_ARGS__)
#define ESP_LOGD(tag, ...) host_log("D", tag, __VA_ARGS__)
