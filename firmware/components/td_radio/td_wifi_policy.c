#include "td_wifi.h"
#include <string.h>

/* Well-formed UTF-8 without control characters (U+0000 to U+001F and U+007F), the string rule of link-v2. */
static bool text_valid(const char *s)
{
    const unsigned char *p = (const unsigned char *)s;
    while (*p) {
        if (*p < 0x80) {
            if (*p < 0x20 || *p == 0x7f) {
                return false;
            }
            p++;
            continue;
        }
        size_t extra;
        uint32_t cp;
        uint32_t least;
        if ((*p & 0xe0) == 0xc0) {
            extra = 1, cp = *p & 0x1f, least = 0x80;
        } else if ((*p & 0xf0) == 0xe0) {
            extra = 2, cp = *p & 0x0f, least = 0x800;
        } else if ((*p & 0xf8) == 0xf0) {
            extra = 3, cp = *p & 0x07, least = 0x10000;
        } else {
            return false;
        }
        for (size_t i = 1; i <= extra; i++) {
            if ((p[i] & 0xc0) != 0x80) {
                return false;
            }
            cp = (cp << 6) | (p[i] & 0x3f);
        }
        if (cp < least || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) {
            return false;
        }
        p += extra + 1;
    }
    return true;
}

static bool hex_key(const char *s, size_t len)
{
    if (len != TD_WIFI_PASSWORD_MAX) {
        return false;
    }
    for (size_t i = 0; i < len; i++) {
        const char c = s[i];
        if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'))) {
            return false;
        }
    }
    return true;
}

bool td_wifi_credentials_valid(const char *ssid, const char *password)
{
    if (!ssid || !password) {
        return false;
    }
    const size_t ssid_len = strlen(ssid);
    const size_t password_len = strlen(password);
    if (ssid_len == 0 || ssid_len > TD_WIFI_SSID_MAX || !text_valid(ssid) || !text_valid(password)) {
        return false;
    }
    return password_len == 0 || (password_len >= TD_WIFI_PASSWORD_MIN && password_len < TD_WIFI_PASSWORD_MAX) ||
           hex_key(password, password_len);
}

/* The numbers are wifi_err_reason_t's in ESP-IDF v6.1. td_radio.c checks them against esp_wifi_types.h. */
td_wifi_reason_t td_wifi_reason_from_driver(uint16_t code)
{
    switch (code) {
    case 201: /* WIFI_REASON_NO_AP_FOUND */
    case 212: /* WIFI_REASON_NO_AP_FOUND_IN_RSSI_THRESHOLD */
        return TD_WIFI_REASON_NOT_FOUND;
    case 210: /* WIFI_REASON_NO_AP_FOUND_W_COMPATIBLE_SECURITY */
    case 211: /* WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD */
        return TD_WIFI_REASON_SECURITY;
    case 2:   /* WIFI_REASON_AUTH_EXPIRE */
    case 14:  /* WIFI_REASON_MIC_FAILURE */
    case 15:  /* WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT */
    case 202: /* WIFI_REASON_AUTH_FAIL */
    case 204: /* WIFI_REASON_HANDSHAKE_TIMEOUT */
        return TD_WIFI_REASON_WRONG_PASSWORD;
    case 5:   /* WIFI_REASON_ASSOC_TOOMANY */
    case 203: /* WIFI_REASON_ASSOC_FAIL */
        return TD_WIFI_REASON_REFUSED;
    case 3:   /* WIFI_REASON_AUTH_LEAVE */
    case 4:   /* WIFI_REASON_DISASSOC_DUE_TO_INACTIVITY */
    case 200: /* WIFI_REASON_BEACON_TIMEOUT */
    case 206: /* WIFI_REASON_AP_TSF_RESET */
        return TD_WIFI_REASON_LOST;
    case 205: /* WIFI_REASON_CONNECTION_FAIL */
        return TD_WIFI_REASON_DRIVER;
    default:
        return TD_WIFI_REASON_OTHER;
    }
}

const char *td_wifi_status_name(td_wifi_status_t status)
{
    switch (status) {
    case TD_WIFI_OFF: return "off";
    case TD_WIFI_CONNECTING: return "connecting";
    case TD_WIFI_CONNECTED: return "connected";
    default: return "failed";
    }
}

const char *td_wifi_reason_name(td_wifi_reason_t reason)
{
    switch (reason) {
    case TD_WIFI_REASON_NONE: return NULL;
    case TD_WIFI_REASON_NOT_FOUND: return "not-found";
    case TD_WIFI_REASON_WRONG_PASSWORD: return "wrong-password";
    case TD_WIFI_REASON_NO_IP: return "no-ip";
    case TD_WIFI_REASON_SECURITY: return "security";
    case TD_WIFI_REASON_REFUSED: return "refused";
    case TD_WIFI_REASON_LOST: return "lost";
    case TD_WIFI_REASON_DRIVER: return "driver";
    default: return "other";
    }
}

uint32_t td_wifi_retry_ms(unsigned attempt)
{
    static const uint32_t delays[] = { 1000, 2000, 4000, 8000, 30000 };
    const unsigned last = sizeof delays / sizeof delays[0] - 1;
    return delays[attempt < last ? attempt : last];
}
