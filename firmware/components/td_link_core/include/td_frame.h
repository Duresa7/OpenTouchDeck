#pragma once

#include <stddef.h>
#include <stdint.h>

#define TD_FRAME_VERSION 1
#define TD_FRAME_PAYLOAD_MAX 1024
#define TD_FRAME_CHUNK_HEADER 3
#define TD_FRAME_CHUNK_DATA_MAX (TD_FRAME_PAYLOAD_MAX - TD_FRAME_CHUNK_HEADER)
#define TD_FRAME_BODY_MAX (6 + TD_FRAME_PAYLOAD_MAX + 2)
#define TD_FRAME_WIRE_MAX (2 * TD_FRAME_BODY_MAX + 2)
#define TD_FRAME_END 0xc0
#define TD_FRAME_ESC 0xdb
#define TD_FRAME_RESPONSE 1
#define TD_FRAME_EVENT 2

typedef enum {
    TD_FRAME_MESSAGE = 1,
    TD_FRAME_CHUNK = 2,
    TD_FRAME_ACK = 3,
} td_frame_type_t;

typedef struct {
    uint8_t type;
    uint8_t seq;
    uint8_t flags;
    uint16_t len;
    const uint8_t *payload;
} td_frame_t;

typedef enum {
    TD_FRAME_OK,
    TD_FRAME_BAD_LENGTH,
    TD_FRAME_BAD_VERSION,
    TD_FRAME_BAD_CRC,
    TD_FRAME_BAD_ESCAPE,
    TD_FRAME_BAD_HEADER,
} td_frame_result_t;

/* The decoded payload borrows body until the caller reuses it. */
td_frame_result_t td_frame_decode(const uint8_t *body, size_t len, td_frame_t *frame);
td_frame_result_t td_frame_decode_slip(const uint8_t *wire, size_t len,
                                     uint8_t *body, size_t cap, td_frame_t *frame);
/* The frame body without SLIP: header, payload and CRC, 8 + len bytes, for carriers that delimit frames themselves.
 * Returns 0 for an invalid frame or when cap is too small. */
size_t td_frame_encode_body(const td_frame_t *frame, uint8_t *body, size_t cap);
size_t td_frame_encode(const td_frame_t *frame, uint8_t *wire, size_t cap);
