#include "td_device.h"
#include "nvs.h"
#include "nvs_flash.h"
#include <stdint.h>
#include <string.h>

esp_err_t td_device_name_get(char out[TD_DEVICE_NAME_MAX + 1])
{
    strcpy(out, "Open TouchDeck");
    esp_err_t init = nvs_flash_init();
    if (init != ESP_OK) return init;
    nvs_handle_t handle;
    esp_err_t err = nvs_open("td_device", NVS_READONLY, &handle);
    if (err == ESP_ERR_NVS_NOT_FOUND) return ESP_OK;
    if (err != ESP_OK) return err;
    char value[TD_DEVICE_NAME_MAX + 1];
    size_t size = sizeof value;
    err = nvs_get_str(handle, "name", value, &size);
    nvs_close(handle);
    if (err == ESP_OK && size == strlen(value) + 1 && td_device_name_valid(value)) strcpy(out, value);
    if (err == ESP_ERR_NVS_NOT_FOUND || err == ESP_ERR_NVS_INVALID_LENGTH || err == ESP_ERR_NVS_TYPE_MISMATCH) return ESP_OK;
    return err;
}

esp_err_t td_device_name_set(const char *name)
{
    if (!td_device_name_valid(name)) return ESP_ERR_INVALID_ARG;
    char old[TD_DEVICE_NAME_MAX + 1];
    esp_err_t err = td_device_name_get(old);
    if (err != ESP_OK || strcmp(old, name) == 0) return err;
    nvs_handle_t handle;
    err = nvs_open("td_device", NVS_READWRITE, &handle);
    if (err != ESP_OK) return err;
    err = nvs_set_str(handle, "name", name);
    if (err == ESP_OK) err = nvs_commit(handle);
    nvs_close(handle);
    return err;
}

#define IDLE_VERSION 1
#define IDLE_BYTES 10

static void put_u32(uint8_t *p, uint32_t v)
{
    for (int i = 0; i < 4; i++) p[i] = (uint8_t)(v >> (8 * i));
}

static uint32_t get_u32(const uint8_t *p)
{
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

esp_err_t td_device_idle_get(td_idle_settings_t *out)
{
    td_idle_defaults(out);
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) return err;
    nvs_handle_t handle;
    err = nvs_open("td_device", NVS_READONLY, &handle);
    if (err == ESP_ERR_NVS_NOT_FOUND) return ESP_OK;
    if (err != ESP_OK) return err;
    uint8_t blob[IDLE_BYTES];
    size_t size = sizeof blob;
    err = nvs_get_blob(handle, "idle", blob, &size);
    nvs_close(handle);
    if (err == ESP_ERR_NVS_NOT_FOUND || err == ESP_ERR_NVS_INVALID_LENGTH || err == ESP_ERR_NVS_TYPE_MISMATCH) {
        return ESP_OK;
    }
    if (err != ESP_OK) return err;
    const td_idle_settings_t stored = {
        .enabled = blob[1] != 0, .timeout_s = get_u32(blob + 2), .interval_s = get_u32(blob + 6),
    };
    if (size == IDLE_BYTES && blob[0] == IDLE_VERSION && blob[1] <= 1 && td_idle_valid(&stored)) *out = stored;
    return ESP_OK;
}

esp_err_t td_device_idle_set(const td_idle_settings_t *settings)
{
    if (!td_idle_valid(settings)) return ESP_ERR_INVALID_ARG;
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) return err;
    uint8_t blob[IDLE_BYTES] = { IDLE_VERSION, settings->enabled ? 1 : 0 };
    put_u32(blob + 2, settings->timeout_s);
    put_u32(blob + 6, settings->interval_s);
    nvs_handle_t handle;
    err = nvs_open("td_device", NVS_READWRITE, &handle);
    if (err != ESP_OK) return err;
    err = nvs_set_blob(handle, "idle", blob, sizeof blob);
    if (err == ESP_OK) err = nvs_commit(handle);
    nvs_close(handle);
    return err;
}
