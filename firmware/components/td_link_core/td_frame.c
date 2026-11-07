#include "td_frame.h"
#include "td_crc16.h"
#include <stdbool.h>
#include <string.h>

static bool valid_header(const td_frame_t *f)
{
    return f->type >= TD_FRAME_MESSAGE && f->type <= TD_FRAME_ACK
        && f->flags <= TD_FRAME_EVENT && (f->flags != TD_FRAME_EVENT || f->seq == 0)
        && f->len <= TD_FRAME_PAYLOAD_MAX
        && (f->type != TD_FRAME_CHUNK || f->len >= TD_FRAME_CHUNK_HEADER)
        && (f->type != TD_FRAME_ACK || f->len == 3);
}

td_frame_result_t td_frame_decode(const uint8_t *body, size_t len, td_frame_t *frame)
{
    if (!body || len < 8 || len > TD_FRAME_BODY_MAX) {
        return TD_FRAME_BAD_LENGTH;
    }
    if (body[0] != TD_FRAME_VERSION) {
        return TD_FRAME_BAD_VERSION;
    }
    size_t payload_len = body[4] | ((size_t)body[5] << 8);
    if (payload_len > TD_FRAME_PAYLOAD_MAX || len != payload_len + 8) {
        return TD_FRAME_BAD_LENGTH;
    }
    uint16_t crc = body[len - 2] | ((uint16_t)body[len - 1] << 8);
    if (crc != td_crc16(body, len - 2)) {
        return TD_FRAME_BAD_CRC;
    }
    td_frame_t decoded = {
        .type = body[1], .seq = body[2], .flags = body[3],
        .len = (uint16_t)payload_len, .payload = body + 6,
    };
    if (!valid_header(&decoded)) {
        return TD_FRAME_BAD_HEADER;
    }
    *frame = decoded;
    return TD_FRAME_OK;
}

td_frame_result_t td_frame_decode_slip(const uint8_t *wire, size_t len,
                                     uint8_t *body, size_t cap, td_frame_t *frame)
{
    if (len < 2 || wire[0] != TD_FRAME_END || wire[len - 1] != TD_FRAME_END) {
        return TD_FRAME_BAD_LENGTH;
    }
    size_t used = 0;
    for (size_t i = 1; i < len - 1; i++) {
        uint8_t c = wire[i];
        if (c == TD_FRAME_END) {
            return TD_FRAME_BAD_ESCAPE;
        }
        if (c == TD_FRAME_ESC) {
            if (++i >= len - 1 || (wire[i] != 0xdc && wire[i] != 0xdd)) {
                return TD_FRAME_BAD_ESCAPE;
            }
            c = wire[i] == 0xdc ? TD_FRAME_END : TD_FRAME_ESC;
        }
        if (used == cap) {
            return TD_FRAME_BAD_LENGTH;
        }
        body[used++] = c;
    }
    return td_frame_decode(body, used, frame);
}

size_t td_frame_encode_body(const td_frame_t *frame, uint8_t *body, size_t cap)
{
    if (!valid_header(frame) || (frame->len && !frame->payload) || cap < 6u + frame->len + 2u) {
        return 0;
    }
    const uint8_t header[6] = {
        TD_FRAME_VERSION, frame->type, frame->seq, frame->flags, frame->len & 0xff, frame->len >> 8,
    };
    memcpy(body, header, sizeof header);
    if (frame->len) {
        memcpy(body + 6, frame->payload, frame->len);
    }
    size_t len = 6 + frame->len;
    uint16_t crc = td_crc16(body, len);
    body[len++] = crc & 0xff;
    body[len++] = crc >> 8;
    return len;
}

size_t td_frame_encode(const td_frame_t *frame, uint8_t *wire, size_t cap)
{
    uint8_t body[TD_FRAME_BODY_MAX];
    const size_t len = td_frame_encode_body(frame, body, sizeof body);
    if (!len) {
        return 0;
    }
    size_t needed = len + 2;
    for (size_t i = 0; i < len; i++) {
        needed += body[i] == TD_FRAME_END || body[i] == TD_FRAME_ESC;
    }
    if (cap < needed) {
        return 0;
    }
    size_t used = 0;
    wire[used++] = TD_FRAME_END;
    for (size_t i = 0; i < len; i++) {
        if (body[i] == TD_FRAME_END || body[i] == TD_FRAME_ESC) {
            wire[used++] = TD_FRAME_ESC;
            wire[used++] = body[i] == TD_FRAME_END ? 0xdc : 0xdd;
        } else {
            wire[used++] = body[i];
        }
    }
    wire[used++] = TD_FRAME_END;
    return used;
}
