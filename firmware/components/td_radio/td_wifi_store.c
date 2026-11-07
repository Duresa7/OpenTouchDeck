#include "td_wifi.h"
#include <string.h>
#include "nvs.h"
#include "nvs_flash.h"

/*
 * The network is one NVS blob, so a power cut keeps the whole old network or the whole new one: version 1, the SSID's
 * length and bytes, then the password's length and bytes. It lives in namespace `td_wifi`, apart from the Device name,
 * the Layouts and the Pairings, so a Layout wipe or a recovery boot keeps it. NVS isn't encrypted on this board, so
 * someone holding the board and a flash reader can read it, as with any ESP-IDF Wi-Fi station.
 */
#define NAMESPACE "td_wifi"
#define KEY "net"
#define VERSION 1
#define BLOB_MAX (3 + TD_WIFI_SSID_MAX + TD_WIFI_PASSWORD_MAX)

static void wipe(void *p, size_t n)
{
    volatile unsigned char *b = p;
    while (n--) {
        *b++ = 0;
    }
}

esp_err_t td_wifi_store_load(char ssid[TD_WIFI_SSID_MAX + 1], char password[TD_WIFI_PASSWORD_MAX + 1])
{
    ssid[0] = '\0';
    password[0] = '\0';
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) {
        return err;
    }
    nvs_handle_t handle;
    err = nvs_open(NAMESPACE, NVS_READONLY, &handle);
    if (err == ESP_ERR_NVS_NOT_FOUND) {
        return ESP_ERR_NOT_FOUND;
    }
    if (err != ESP_OK) {
        return err;
    }
    unsigned char blob[BLOB_MAX];
    size_t size = sizeof blob;
    err = nvs_get_blob(handle, KEY, blob, &size);
    nvs_close(handle);
    if (err == ESP_ERR_NVS_NOT_FOUND || err == ESP_ERR_NVS_INVALID_LENGTH || err == ESP_ERR_NVS_TYPE_MISMATCH) {
        err = ESP_ERR_NOT_FOUND;
    }
    if (err == ESP_OK) {
        const size_t ssid_len = size >= 2 ? blob[1] : 0;
        const size_t password_len = size >= 3 + ssid_len ? blob[2 + ssid_len] : 0;
        err = ESP_ERR_NOT_FOUND;
        if (size >= 3 && blob[0] == VERSION && ssid_len <= TD_WIFI_SSID_MAX && password_len <= TD_WIFI_PASSWORD_MAX &&
            size == 3 + ssid_len + password_len) {
            memcpy(ssid, blob + 2, ssid_len);
            ssid[ssid_len] = '\0';
            memcpy(password, blob + 3 + ssid_len, password_len);
            password[password_len] = '\0';
            err = td_wifi_credentials_valid(ssid, password) ? ESP_OK : ESP_ERR_NOT_FOUND;
        }
    }
    wipe(blob, sizeof blob);
    if (err != ESP_OK) {
        wipe(password, TD_WIFI_PASSWORD_MAX + 1);
        ssid[0] = '\0';
    }
    return err;
}

esp_err_t td_wifi_store_save(const char *ssid, const char *password)
{
    if (!td_wifi_credentials_valid(ssid, password)) {
        return ESP_ERR_INVALID_ARG;
    }
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) {
        return err;
    }
    const size_t ssid_len = strlen(ssid);
    const size_t password_len = strlen(password);
    unsigned char blob[BLOB_MAX];
    blob[0] = VERSION;
    blob[1] = (unsigned char)ssid_len;
    memcpy(blob + 2, ssid, ssid_len);
    blob[2 + ssid_len] = (unsigned char)password_len;
    memcpy(blob + 3 + ssid_len, password, password_len);
    nvs_handle_t handle;
    err = nvs_open(NAMESPACE, NVS_READWRITE, &handle);
    if (err == ESP_OK) {
        err = nvs_set_blob(handle, KEY, blob, 3 + ssid_len + password_len);
        if (err == ESP_OK) {
            err = nvs_commit(handle);
        }
        nvs_close(handle);
    }
    wipe(blob, sizeof blob);
    return err;
}

esp_err_t td_wifi_store_erase(void)
{
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) {
        return err;
    }
    nvs_handle_t handle;
    err = nvs_open(NAMESPACE, NVS_READWRITE, &handle);
    if (err != ESP_OK) {
        return err;
    }
    err = nvs_erase_key(handle, KEY);
    if (err == ESP_ERR_NVS_NOT_FOUND) {
        err = ESP_OK;
    }
    if (err == ESP_OK) {
        err = nvs_commit(handle);
    }
    nvs_close(handle);
    return err;
}
