#include "td_transfer.h"
#include <stdio.h>
#include <string.h>
#include "td_frame.h"

#define GET_IDLE_MS 5000

void td_transfer_init(td_transfer_t *t, const td_transfer_ports_t *ports)
{
    *t = (td_transfer_t){ .ports = ports };
}

/* Any bulk open, v1 or v2, ends the replay of the last Asset or picture end. */
static void bulk_opened(td_transfer_t *t)
{
    t->asset_end.xfer = 0;
    t->photo_end.xfer = 0;
}

void td_transfer_drop(td_transfer_t *t)
{
    if (t->kind == TD_TRANSFER_ASSET_PUT) {
        t->ports->asset_abort(t->ports->ctx);
    }
    if (t->buf != NULL) {
        t->ports->release(t->ports->ctx, t->buf);
    }
    t->buf = NULL;
    t->fill = 0;
    t->kind = TD_TRANSFER_NONE;
}

void td_transfer_close(td_transfer_t *t)
{
    if (t->kind == TD_TRANSFER_UPDATE) {
        t->ports->update_cancel(t->ports->ctx);
    }
    td_transfer_drop(t);
}

/* Takes the next xfer id, stamps the activity clock and claims the origin: the carrier connection of the request being
 * handled. */
static void opened(td_transfer_t *t, td_transfer_kind_t kind)
{
    t->kind = kind;
    t->last_id = t->last_id % 255 + 1;
    t->last_ms = t->ports->now_ms(t->ports->ctx);
    t->origin = t->ports->origin(t->ports->ctx);
}

bool td_transfer_begin_buffered(td_transfer_t *t, td_transfer_kind_t kind, size_t bytes)
{
    td_transfer_close(t);
    bulk_opened(t);
    t->buf = t->ports->alloc(t->ports->ctx, bytes);
    if (t->buf == NULL) {
        return false;
    }
    opened(t, kind);
    return true;
}

void td_transfer_begin_streamed(td_transfer_t *t, td_transfer_kind_t kind)
{
    td_transfer_drop(t);
    bulk_opened(t);
    t->asset_failure = NULL;
    opened(t, kind);
}

static bool put_sink(void *ctx, const uint8_t *data, size_t len)
{
    td_transfer_t *t = ctx;
    memcpy(t->buf + t->fill, data, len);
    t->fill += len;
    return true;
}

static bool update_sink(void *ctx, const uint8_t *data, size_t len)
{
    const td_transfer_t *t = ctx;
    return t->ports->update_write(t->ports->ctx, data, len);
}

static bool asset_sink(void *ctx, const uint8_t *data, size_t len)
{
    td_transfer_t *t = ctx;
    const char *failure = t->ports->asset_write(t->ports->ctx, data, len);
    if (failure != NULL) {
        t->asset_failure = failure;
        return false;
    }
    return true;
}

void td_transfer_arm(td_transfer_t *t, bool sending, uint32_t bytes, uint32_t crc32)
{
    td_xfer_sink_t sink = NULL;
    switch (t->kind) {
    case TD_TRANSFER_LAYOUT_PUT:
    case TD_TRANSFER_ICON_PUT:
    case TD_TRANSFER_PHOTO_PUT:
        sink = put_sink;
        break;
    case TD_TRANSFER_UPDATE:
        sink = update_sink;
        break;
    case TD_TRANSFER_ASSET_PUT:
        sink = asset_sink;
        break;
    case TD_TRANSFER_GET:
    case TD_TRANSFER_NONE:
        break;
    }
    td_xfer_init(&t->x, t->last_id, sending, bytes, crc32, sink, t);
}

/* True while the frame being handled arrived on the connection that opened the transfer. */
static bool ours(const td_transfer_t *t)
{
    const uint32_t current = t->ports->origin(t->ports->ctx);
    return current != 0 && t->origin == current;
}

bool td_transfer_other_origin(const td_transfer_t *t)
{
    return t->kind != TD_TRANSFER_NONE && !ours(t);
}

void td_transfer_select(td_transfer_t *t)
{
    if (t->kind == TD_TRANSFER_ASSET_PUT || t->kind == TD_TRANSFER_PHOTO_PUT) {
        td_transfer_drop(t);
    }
}

bool td_transfer_photo_cancelled(td_transfer_t *t)
{
    if (t->kind != TD_TRANSFER_PHOTO_PUT || t->ports->photo_idle(t->ports->ctx, t->photo_entry)) {
        return false;
    }
    t->photo_end.xfer = t->x.id;
    t->photo_end.failure = "idle";
    td_transfer_drop(t);
    return true;
}

void td_transfer_asset_end_record(td_transfer_t *t, uint8_t xfer, const char *reason, const char *id, uint32_t bytes)
{
    t->asset_end.xfer = xfer;
    t->asset_end.reason = reason;
    t->asset_end.bytes = bytes;
    snprintf(t->asset_end.id, sizeof t->asset_end.id, "%s", id ? id : "");
}

void td_transfer_chunk(td_transfer_t *t, uint8_t seq, const uint8_t *payload, size_t len)
{
    /* A transfer moves only on the carrier connection that opened it. */
    if (!ours(t) || len < TD_FRAME_CHUNK_HEADER
        || (t->kind != TD_TRANSFER_LAYOUT_PUT && t->kind != TD_TRANSFER_ICON_PUT && t->kind != TD_TRANSFER_UPDATE
            && t->kind != TD_TRANSFER_ASSET_PUT && t->kind != TD_TRANSFER_PHOTO_PUT)
        || payload[0] != t->x.id) {
        t->ports->count_drop(t->ports->ctx);
        return;
    }
    if (td_transfer_photo_cancelled(t)) {
        return;
    }
    const uint16_t idx = (uint16_t)(payload[1] | (payload[2] << 8));
    /* A duplicate, a gap and a chunk past the window all get the same answer: the next index the deck wants. */
    td_xfer_receive(&t->x, idx, payload + TD_FRAME_CHUNK_HEADER, len - TD_FRAME_CHUNK_HEADER);
    t->last_ms = t->ports->now_ms(t->ports->ctx);
    if (t->kind == TD_TRANSFER_ASSET_PUT && t->asset_failure != NULL) {
        /* A failed sink ends the transfer and stops the acks. v2.asset.end reports why. */
        td_transfer_asset_end_record(t, t->x.id, t->asset_failure, NULL, 0);
        td_transfer_drop(t);
        return;
    }
    const uint8_t ack[3] = { t->x.id, t->x.next & 0xff, t->x.next >> 8 };
    t->ports->send(t->ports->ctx, TD_FRAME_ACK, seq, TD_FRAME_RESPONSE, ack, sizeof ack);
}

void td_transfer_ack(td_transfer_t *t, const uint8_t *payload, size_t len)
{
    if (!ours(t) || len != 3 || t->kind != TD_TRANSFER_GET || payload[0] != t->x.id) {
        t->ports->count_drop(t->ports->ctx);
        return;
    }
    td_xfer_ack(&t->x, (uint16_t)(payload[1] | (payload[2] << 8)));
    t->last_ms = t->ports->now_ms(t->ports->ctx);
}

static void send_chunk(td_transfer_t *t, uint16_t idx)
{
    static uint8_t payload[TD_FRAME_PAYLOAD_MAX];
    const size_t start = (size_t)idx * TD_FRAME_CHUNK_DATA_MAX;
    size_t len = t->x.bytes - start;
    if (len > TD_FRAME_CHUNK_DATA_MAX) {
        len = TD_FRAME_CHUNK_DATA_MAX;
    }
    payload[0] = t->x.id;
    payload[1] = idx & 0xff;
    payload[2] = idx >> 8;
    memcpy(payload + TD_FRAME_CHUNK_HEADER, t->buf + start, len);
    t->tx_seq = t->tx_seq % 255 + 1;
    t->ports->send(t->ports->ctx, TD_FRAME_CHUNK, t->tx_seq, 0, payload, TD_FRAME_CHUNK_HEADER + len);
}

void td_transfer_service(td_transfer_t *t, uint32_t now_ms)
{
    if (t->kind == TD_TRANSFER_NONE || td_transfer_photo_cancelled(t)) {
        return;
    }
    if (!t->ports->origin_live(t->ports->ctx, t->origin)) {
        /* The carrier connection that opened it ended: a network session closed. */
        td_transfer_close(t);
        return;
    }
    if (t->kind == TD_TRANSFER_GET) {
        int idx;
        for (int guard = 0; guard < 2 * TD_XFER_WINDOW && (idx = td_xfer_due(&t->x, now_ms)) >= 0; guard++) {
            send_chunk(t, (uint16_t)idx);
        }
        if (t->x.aborted || now_ms - t->last_ms > GET_IDLE_MS) {
            td_transfer_close(t);
        }
    } else if (now_ms - t->last_ms > TD_XFER_IDLE_MS) {
        td_transfer_close(t);
    }
}
