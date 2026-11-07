#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "esp_err.h"
#include "td_frame.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A carrier moves whole link frames between the Device and the Companion, each its own way. USB moves them as SLIP
 * over the USB-Serial/JTAG console, and td_link_start adds it first. The radio build adds net-v2's TCP carrier.
 *
 * Online state is kept per carrier: a carrier goes online when a `hello` arrives on it, and offline when nothing has
 * arrived on it for 5 seconds or when it reports its connection gone. Replies and ACKs (TD_FRAME_RESPONSE) leave on
 * the carrier their request arrived on. Events and the Device's chunks leave on one active carrier: the first carrier
 * added that is online, or, while none is, the one the last handled frame arrived on. So USB wins whenever the
 * Companion talks over it.
 *
 * A network carrier gets only the messages that are safe over a network: the rest answer `reason:"usb-only"`.
 */
typedef struct td_link_carrier td_link_carrier_t;

struct td_link_carrier {
    /* Sends one frame, on the td_link task. Returns false when the frame did not go out whole. A carrier with state
     * of its own embeds this struct and finds its state from the pointer. */
    bool (*send)(const td_link_carrier_t *carrier, const td_frame_t *frame);
    /* What link.state, hello and the console call it: "usb", "wifi", as net.state names carriers. */
    const char *name;
    /* Frames arrive from a network, so USB-only messages are refused on it. */
    bool network;
};

#define TD_LINK_CARRIERS_MAX 3

/*
 * Adds a carrier after td_link_start, behind the carriers already added. The carrier must stay valid for good. Call it
 * from a task, never from an ISR. Returns ESP_OK, ESP_ERR_INVALID_ARG for a NULL carrier or one without `send`,
 * ESP_ERR_INVALID_STATE before td_link_start or when it was added already, and ESP_ERR_NO_MEM when
 * TD_LINK_CARRIERS_MAX are in use.
 */
esp_err_t td_link_carrier_add(const td_link_carrier_t *carrier);

/*
 * Hands the link one frame body, SLIP already removed, that arrived on `carrier`. It has the td_rx_frame_fn shape,
 * with the carrier as its context. Any task, without blocking for long. A NULL body counts a malformed frame.
 */
void td_link_carrier_receive(const uint8_t *body, size_t len, void *carrier);

/* The carrier's connection ended, so it goes offline now rather than after the idle time. Any task. */
void td_link_carrier_offline(const td_link_carrier_t *carrier);

/* Counts the connections the carrier reported gone with td_link_carrier_offline. Any task. */
uint32_t td_link_carrier_generation(const td_link_carrier_t *carrier);

/* Inside a carrier's send: the generation of the connection the frame belongs to. A carrier that queues frames
 * stores it with each one and drops those whose generation is no longer td_link_carrier_generation, so a reply to an
 * ended session never goes out on the next one. */
uint32_t td_link_send_generation(void);

/* True while the carrier is online: a hello on it, and a frame within the last 5 seconds. Any task. */
bool td_link_carrier_online(const td_link_carrier_t *carrier);

/* True while USB is online: a hello on it, and a frame within the last 5 seconds. Any task. */
bool td_link_usb_online(void);

#ifdef __cplusplus
}
#endif
