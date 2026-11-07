#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "cJSON.h"
#include "esp_console.h"
#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The radio build's Wi-Fi station: one stored network, joined as a station and joined again after every restart.
 *
 * The network is set over USB only, with the `wifi.*` link messages or the `wifi` console command, and kept in NVS
 * (td_wifi_store). The password never leaves the Device again: no reply, log line or console echo carries it.
 *
 * The station logic here is plain C over a small driver interface, so the host tests run it with a fake driver and a
 * fake NVS. td_radio.c is the ESP-IDF driver: it turns Wi-Fi and IP events into the td_wifi_on_* calls below.
 */

#define TD_WIFI_SSID_MAX 32
/* A WPA passphrase is 8 to 63 printable ASCII characters. 64 hex digits are a raw pre-shared key. */
#define TD_WIFI_PASSWORD_MIN 8
#define TD_WIFI_PASSWORD_MAX 64
/* How long an associated station waits for an address before it gives up with `no-ip`. */
#define TD_WIFI_DHCP_MS 15000

typedef enum {
    /* No network is stored. */
    TD_WIFI_OFF,
    /* A network is stored and the first join since it was set, or since boot, has not finished. */
    TD_WIFI_CONNECTING,
    /* Joined, with an address. */
    TD_WIFI_CONNECTED,
    /* The last join failed or the connection dropped. The station retries by itself; `reason` says why. */
    TD_WIFI_FAILED,
} td_wifi_status_t;

typedef enum {
    TD_WIFI_REASON_NONE,
    /* No access point with that name answered. */
    TD_WIFI_REASON_NOT_FOUND,
    /* The access point refused the password, or the handshake that checks it timed out. */
    TD_WIFI_REASON_WRONG_PASSWORD,
    /* Joined, but no address came within TD_WIFI_DHCP_MS, or the address was lost. */
    TD_WIFI_REASON_NO_IP,
    /* The network uses a security mode this station doesn't support, such as WEP or an enterprise network. */
    TD_WIFI_REASON_SECURITY,
    /* The access point refused the association, for example because it is full. */
    TD_WIFI_REASON_REFUSED,
    /* The connection was up and then dropped, for example when the access point went silent. */
    TD_WIFI_REASON_LOST,
    /* The Wi-Fi driver did not start or refused to start a join. */
    TD_WIFI_REASON_DRIVER,
    TD_WIFI_REASON_OTHER,
} td_wifi_reason_t;

typedef struct {
    bool configured;
    char ssid[TD_WIFI_SSID_MAX + 1];
    td_wifi_status_t status;
    td_wifi_reason_t reason;
    /* A dotted IPv4 address while connected, empty otherwise. */
    char ip[16];
    /* The signal in dBm while connected. */
    int rssi;
    bool has_rssi;
} td_wifi_state_t;

/* ---- Policy: pure functions, no state ---- */

/* The SSID is 1 to 32 bytes of UTF-8 without control characters. The password is empty for an open network, 8 to 63
 * printable ASCII characters (0x20 to 0x7e), or 64 hex digits. */
bool td_wifi_credentials_valid(const char *ssid, const char *password);
/* What a disconnect with this ESP-IDF reason code (wifi_err_reason_t) tells the person. */
td_wifi_reason_t td_wifi_reason_from_driver(uint16_t code);
/* The ESP-IDF reason code of the station leaving on its own request. The station logic ignores those disconnects. */
#define TD_WIFI_DRIVER_ASSOC_LEAVE 8
const char *td_wifi_status_name(td_wifi_status_t status);
/* The reason as the link reports it, NULL for TD_WIFI_REASON_NONE. */
const char *td_wifi_reason_name(td_wifi_reason_t reason);
/* The wait before join attempt `attempt` + 1 after a failure: 1, 2, 4 and 8 seconds, then 30 seconds for good. */
uint32_t td_wifi_retry_ms(unsigned attempt);

/* ---- Storage: NVS namespace `td_wifi`, key `net` ---- */

/* Reads the stored network. ESP_ERR_NOT_FOUND when none is stored or the stored one is unusable. */
esp_err_t td_wifi_store_load(char ssid[TD_WIFI_SSID_MAX + 1], char password[TD_WIFI_PASSWORD_MAX + 1]);
/* Replaces the stored network in one NVS write and commits it. A failure keeps the old one. */
esp_err_t td_wifi_store_save(const char *ssid, const char *password);
/* Erases the stored network and commits. Erasing nothing is ESP_OK. */
esp_err_t td_wifi_store_erase(void);

/* ---- The station ---- */

/* What the station logic asks of the Wi-Fi driver. Every call is made with the station lock held. */
typedef struct {
    /* Sets the station's network and starts a join. False when the driver refused. */
    bool (*join)(const char *ssid, const char *password);
    /* Leaves the network or stops a join. The disconnect it causes comes back as reason TD_WIFI_DRIVER_ASSOC_LEAVE. */
    void (*leave)(void);
    /* Starts the one-shot retry or DHCP timer, replacing the timer of that kind. */
    void (*retry_in)(uint32_t ms);
    void (*dhcp_in)(uint32_t ms);
    /* Stops both timers. */
    void (*stop_timers)(void);
    /* The signal of the joined access point. False when the driver can't say. May be NULL. */
    bool (*rssi)(int *dbm);
} td_wifi_driver_t;

/* Loads the stored network, once, before the station starts. A stored network shows as `connecting`. */
esp_err_t td_wifi_init(const td_wifi_driver_t *driver);
/* Sets the ESP-IDF log tags whose INFO and WARN lines name the network, a nearby access point or the address to
 * errors only: the Wi-Fi driver `wifi` ("connected with <SSID>, bssid=..."), the supplicant `wpa` and the netif handler
 * `esp_netif_handlers` ("sta ip: ..."). td_radio calls it before esp_wifi_init. */
void td_wifi_quiet_logs(void);
/* The station started, or failed to start with `err`. A started station joins the stored network. */
void td_wifi_started(esp_err_t err);

/* Stores the network and joins it, replacing any other. ESP_ERR_INVALID_ARG for credentials td_wifi_credentials_valid
 * refuses, ESP_ERR_INVALID_STATE before td_wifi_init, or the NVS error, and then nothing changed. */
esp_err_t td_wifi_set(const char *ssid, const char *password);
/* Erases the stored network and leaves it. */
esp_err_t td_wifi_forget(void);
/* A snapshot, from any task. */
td_wifi_state_t td_wifi_state(void);
/* The status alone, without the lock, for the indicator. */
td_wifi_status_t td_wifi_status(void);

/* Driver events, in the order the driver sees them. */
void td_wifi_on_associated(void);
void td_wifi_on_disconnected(uint16_t driver_reason, const uint8_t *ssid, size_t ssid_len);
void td_wifi_on_got_ip(const char *ip);
void td_wifi_on_lost_ip(void);
void td_wifi_on_retry(void);
void td_wifi_on_dhcp_timeout(void);

/* ---- Messages ---- */

/* The `wifi.*` link messages, for td_link_wifi_register. */
void td_wifi_link_message(const char *message, const cJSON *request, cJSON *reply);
/* Adds the state fields of the link replies: configured, ssid, state, reason, ip and rssi. Never the password. */
void td_wifi_state_fields(cJSON *reply, const td_wifi_state_t *state);
/* The `wifi` console command: state, set <ssid> <password>, forget. */
const esp_console_cmd_t *td_wifi_console_commands(size_t *count);

#ifdef __cplusplus
}
#endif
