#include "td_netlink.h"
#include <string.h>

static void to_hex(char *out, const uint8_t *in, size_t len)
{
    static const char digits[] = "0123456789abcdef";
    for (size_t i = 0; i < len; i++) {
        out[2 * i] = digits[in[i] >> 4];
        out[2 * i + 1] = digits[in[i] & 15];
    }
    out[2 * len] = '\0';
}

bool td_netlink_hint(const uint8_t ltk[TD_NETLINK_KEY_LEN], const uint8_t nonce[TD_NETLINK_ADVERT_NONCE_LEN],
                     uint8_t hint[TD_NETLINK_HINT_LEN])
{
    uint8_t key[TD_NETLINK_KEY_LEN], mac[TD_NETLINK_H_LEN];
    const td_netlink_part_t parts[] = { { "hint", 4 }, { nonce, TD_NETLINK_ADVERT_NONCE_LEN } };
    const bool ok = td_netlink_derive(ltk, "discovery", key) && td_netlink_hmac(key, sizeof key, parts, 2, mac);
    if (ok) memcpy(hint, mac, TD_NETLINK_HINT_LEN);
    else memset(hint, 0, TD_NETLINK_HINT_LEN);
    td_netlink_wipe(key, sizeof key);
    td_netlink_wipe(mac, sizeof mac);
    return ok;
}

bool td_netlink_pairing_mode(const td_netlink_deck_t *deck)
{
    return deck->window.open && !deck->locked;
}

bool td_netlink_advert_build(const td_netlink_deck_t *deck, const uint8_t nonce[TD_NETLINK_ADVERT_NONCE_LEN],
                             const uint8_t fillers[TD_NETLINK_HINT_SLOTS * TD_NETLINK_HINT_LEN], const char *name,
                             td_netlink_advert_t *out)
{
    memset(out, 0, sizeof *out);
    to_hex(out->label, nonce, 4);
    memcpy(out->instance, "TouchDeck ", 10);
    memcpy(out->instance + 10, out->label, sizeof out->label);
    memcpy(out->host, "td-", 3);
    memcpy(out->host + 3, out->label, sizeof out->label);
    to_hex(out->n, nonce, TD_NETLINK_ADVERT_NONCE_LEN);

    /* One hint per stored Pairing in slot order, then a random filler per free slot, so the count doesn't show. */
    uint8_t hints[TD_NETLINK_HINT_SLOTS * TD_NETLINK_HINT_LEN];
    memcpy(hints, fillers, sizeof hints);
    bool ok = true;
    for (int i = 0; i < deck->pairings.count && i < TD_NETLINK_HINT_SLOTS; i++) {
        ok = td_netlink_hint(deck->pairings.entries[i].ltk, nonce, hints + i * TD_NETLINK_HINT_LEN) && ok;
    }
    to_hex(out->h, hints, sizeof hints);
    if (!ok) {
        return false;
    }

    /* The id, name and count only while a person is pairing on purpose. */
    out->pairing = td_netlink_pairing_mode(deck);
    if (out->pairing) {
        to_hex(out->id, deck->device_id, TD_NETLINK_DEVICE_ID_LEN);
        size_t len = name ? strlen(name) : 0;
        if (len >= sizeof out->name) {
            /* Cut on a UTF-8 character boundary. */
            len = sizeof out->name - 1;
            while (len > 0 && ((uint8_t)name[len] & 0xc0) == 0x80) {
                len--;
            }
        }
        if (len == 0 || !td_netlink_name_valid((const uint8_t *)name, len)) {
            memcpy(out->name, "TouchDeck", sizeof "TouchDeck");
        } else {
            memcpy(out->name, name, len);
            out->name[len] = '\0';
        }
        out->paired[0] = (char)('0' + (deck->pairings.count <= TD_NETLINK_PAIRINGS_MAX ? deck->pairings.count : 0));
    }
    return true;
}

size_t td_netlink_advert_txt(const td_netlink_advert_t *advert, td_netlink_txt_t txt[TD_NETLINK_ADVERT_TXT_MAX])
{
    size_t count = 0;
    txt[count++] = (td_netlink_txt_t){ "proto", "2" };
    txt[count++] = (td_netlink_txt_t){ "n", advert->n };
    txt[count++] = (td_netlink_txt_t){ "h", advert->h };
    if (advert->pairing) {
        txt[count++] = (td_netlink_txt_t){ "id", advert->id };
        txt[count++] = (td_netlink_txt_t){ "name", advert->name };
        txt[count++] = (td_netlink_txt_t){ "paired", advert->paired };
    }
    return count;
}

bool td_netlink_ble_hint(const uint8_t ltk[TD_NETLINK_KEY_LEN], const uint8_t nonce[TD_NETLINK_BLE_NONCE_LEN],
                        uint8_t hint[TD_NETLINK_BLE_HINT_LEN])
{
    uint8_t key[TD_NETLINK_KEY_LEN], mac[TD_NETLINK_H_LEN];
    const td_netlink_part_t parts[] = { { "ble-hint", 8 }, { nonce, TD_NETLINK_BLE_NONCE_LEN } };
    const bool ok = td_netlink_derive(ltk, "discovery", key) && td_netlink_hmac(key, sizeof key, parts, 2, mac);
    if (ok) memcpy(hint, mac, TD_NETLINK_BLE_HINT_LEN);
    else memset(hint, 0, TD_NETLINK_BLE_HINT_LEN);
    td_netlink_wipe(key, sizeof key);
    td_netlink_wipe(mac, sizeof mac);
    return ok;
}

bool td_netlink_ble_advert_build(const td_netlink_deck_t *deck,
                                const uint8_t nonce[TD_NETLINK_BLE_NONCE_LEN],
                                const uint8_t filler[TD_NETLINK_BLE_HINT_LEN], unsigned slot,
                                uint8_t out[TD_NETLINK_BLE_DISCOVERY_LEN])
{
    memset(out, 0, TD_NETLINK_BLE_DISCOVERY_LEN);
    if (slot >= TD_NETLINK_HINT_SLOTS || deck->pairings.count > TD_NETLINK_PAIRINGS_MAX) return false;
    if (td_netlink_pairing_mode(deck)) {
        out[0] = 0x82;
        memcpy(out + 1, deck->device_id, TD_NETLINK_DEVICE_ID_LEN);
        out[7] = deck->pairings.count;
        memcpy(out + 8, filler, 5);
        return true;
    }
    out[0] = 2;
    memcpy(out + 1, nonce, TD_NETLINK_BLE_NONCE_LEN);
    memcpy(out + 7, filler, TD_NETLINK_BLE_HINT_LEN);
    if (slot < deck->pairings.count && !td_netlink_ble_hint(deck->pairings.entries[slot].ltk, nonce, out + 7)) {
        memset(out, 0, TD_NETLINK_BLE_DISCOVERY_LEN);
        return false;
    }
    return true;
}
