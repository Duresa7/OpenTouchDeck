#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "td_xfer.h"

/*
 * The one open transfer of a link, and everything that must be released with it.
 *
 * A transfer belongs to the Carrier connection whose request opened it (its origin). The transfer owns what it holds:
 * the PSRAM buffer of a put or a get, an Asset upload in flight, a firmware update that is still receiving. It
 * releases them when it is replaced, finishes, goes idle, or its connection ends. Handlers (td_link_files.c) check a
 * request, decide whether to open one, and call begin; the chunk algorithm itself stays in td_xfer.
 *
 * Effects go through td_transfer_ports_t, so the host can replace the heap, the update and Asset writers, the clock,
 * the wire and the carrier session, and run the production transfer code. Every function runs on the link task.
 */

typedef enum {
    TD_TRANSFER_NONE,
    TD_TRANSFER_LAYOUT_PUT,
    TD_TRANSFER_ICON_PUT,
    TD_TRANSFER_GET,
    TD_TRANSFER_UPDATE,
    TD_TRANSFER_ASSET_PUT,
    TD_TRANSFER_PHOTO_PUT,
} td_transfer_kind_t;

typedef struct {
    /* A buffer of `bytes`, or NULL. Prefers PSRAM on the Device. */
    uint8_t *(*alloc)(void *ctx, size_t bytes);
    void (*release)(void *ctx, uint8_t *buf);
    /* Cut a firmware update that was still receiving. Cut an Asset upload: its temporary file goes. */
    void (*update_cancel)(void *ctx);
    void (*asset_abort)(void *ctx);
    /* The sinks of the two streamed kinds. update_write returns false when the image is refused. asset_write returns
     * NULL when the bytes were taken, else the stable refusal reason. */
    bool (*update_write)(void *ctx, const uint8_t *data, size_t len);
    const char *(*asset_write)(void *ctx, const uint8_t *data, size_t len);
    /* The carrier connection of the frame being handled, or 0 while there is none (td_link_origin_current), and
     * whether that connection still lasts. */
    uint32_t (*origin)(void *ctx);
    bool (*origin_live)(void *ctx, uint32_t origin);
    /* True while the idle face the picture put was admitted under (its `entry`) is still up. */
    bool (*photo_idle)(void *ctx, uint32_t entry);
    /* A chunk or an ack leaves on the active carrier. A frame the link could not use is counted. */
    void (*send)(void *ctx, uint8_t type, uint8_t seq, uint8_t flags, const uint8_t *payload, size_t len);
    void (*count_drop)(void *ctx);
    uint32_t (*now_ms)(void *ctx);
    void *ctx;
} td_transfer_ports_t;

/* The outcome of the last v2 Asset transfer that closed, which v2.asset.end repeats for its xfer until the next bulk
 * open: the committed id and size, or the refusal reason. */
#define TD_TRANSFER_ASSET_ID_MAX 64
typedef struct {
    uint8_t xfer;
    const char *reason;
    char id[TD_TRANSFER_ASSET_ID_MAX + 1];
    uint32_t bytes;
} td_transfer_asset_end_t;

typedef struct {
    uint8_t xfer;
    const char *failure;
} td_transfer_photo_end_t;

typedef struct {
    const td_transfer_ports_t *ports;
    td_transfer_kind_t kind;
    td_xfer_t x;
    /* A put's whole file, or the file a get sends. */
    uint8_t *buf;
    size_t fill;
    uint32_t origin;
    uint32_t last_ms;
    uint8_t last_id;
    uint8_t tx_seq;
    /* The reason the Asset sink refused, until the transfer is dropped. */
    const char *asset_failure;
    /* The idle face a picture put was admitted under. */
    uint32_t photo_entry;
    td_transfer_asset_end_t asset_end;
    td_transfer_photo_end_t photo_end;
} td_transfer_t;

/* A transfer with no kind and nothing held. `ports` must outlive it. */
void td_transfer_init(td_transfer_t *t, const td_transfer_ports_t *ports);

/* Opens a transfer that holds a buffer of `bytes`: a put, or a get. Cuts the open one first, and ends the replay of
 * the last Asset or picture end. False when no buffer is left; the open transfer is gone all the same. Otherwise
 * claims the origin, takes the next xfer id and leaves the transfer to td_transfer_arm. */
bool td_transfer_begin_buffered(td_transfer_t *t, td_transfer_kind_t kind, size_t bytes);

/* Opens a transfer that holds no buffer: an update, whose td_update_begin has accepted, or an Asset put, whose
 * td_asset_upload_begin has. Frees a buffer of another kind, and leaves a running update alone. */
void td_transfer_begin_streamed(td_transfer_t *t, td_transfer_kind_t kind);

/* Starts the chunk algorithm for the transfer just begun, with the kind's sink. `sending` is a get, which has no
 * sink. */
void td_transfer_arm(td_transfer_t *t, bool sending, uint32_t bytes, uint32_t crc32);

/* Frees the buffer, aborts an Asset upload and forgets the transfer. A running update is left alone. */
void td_transfer_drop(td_transfer_t *t);

/* A transfer that is replaced or cut: drop, and an update that was still receiving returns to IDLE. */
void td_transfer_close(td_transfer_t *t);

/* True when a transfer is open that another carrier connection opened: its end request finds no transfer. */
bool td_transfer_other_origin(const td_transfer_t *t);

/* A protocol selection discards an open v2 Asset or picture upload. */
void td_transfer_select(td_transfer_t *t);

/* A picture put whose idle face is gone is cut and its end recorded as "idle". True when it was cut. */
bool td_transfer_photo_cancelled(td_transfer_t *t);

/* Records how the last Asset transfer ended, for v2.asset.end to repeat. */
void td_transfer_asset_end_record(td_transfer_t *t, uint8_t xfer, const char *reason, const char *id, uint32_t bytes);

/* The chunk and ack frames of the frame being handled. Chunk and ack frames from a connection other than the one that
 * opened the transfer are dropped. A chunk of a put is acked with the next index the Device wants. */
void td_transfer_chunk(td_transfer_t *t, uint8_t seq, const uint8_t *payload, size_t len);
void td_transfer_ack(td_transfer_t *t, const uint8_t *payload, size_t len);

/* The periodic work of a transfer: cuts it when its connection ended or it stopped moving, and sends the chunks of
 * a get that are due. */
void td_transfer_service(td_transfer_t *t, uint32_t now_ms);
