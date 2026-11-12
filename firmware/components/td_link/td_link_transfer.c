#include "td_link_priv.h"
#include "esp_heap_caps.h"
#include "esp_timer.h"
#include "power.h"
#include "td_asset.h"
#include "td_update.h"

/* The Device's side of the transfer ports (td_transfer.h): the heap, the firmware update and Asset writers, the
 * carrier session and the link's clock and wire. */

static uint8_t *port_alloc(void *ctx, size_t bytes)
{
    (void)ctx;
    uint8_t *buf = heap_caps_malloc(bytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    return buf != NULL ? buf : heap_caps_malloc(bytes, MALLOC_CAP_8BIT);
}

static void port_release(void *ctx, uint8_t *buf)
{
    (void)ctx;
    heap_caps_free(buf);
}

static void port_update_cancel(void *ctx)
{
    (void)ctx;
    td_update_cancel();
}

static void port_asset_abort(void *ctx)
{
    (void)ctx;
    td_asset_upload_abort();
}

static bool port_update_write(void *ctx, const uint8_t *data, size_t len)
{
    (void)ctx;
    return td_update_write(data, len);
}

static const char *port_asset_write(void *ctx, const uint8_t *data, size_t len)
{
    (void)ctx;
    const td_asset_result_t result = td_asset_upload_write(data, len);
    return result == TD_ASSET_OK ? NULL : td_asset_result_reason(result);
}

static uint32_t port_origin(void *ctx)
{
    (void)ctx;
    return td_link_origin_current();
}

static bool port_origin_live(void *ctx, uint32_t origin)
{
    (void)ctx;
    return td_link_origin_live(origin);
}

static bool port_photo_idle(void *ctx, uint32_t entry)
{
    (void)ctx;
    uint32_t now = 0;
    return power_idle_face(&now) && now == entry;
}

static void port_send(void *ctx, uint8_t type, uint8_t seq, uint8_t flags, const uint8_t *payload, size_t len)
{
    (void)ctx;
    td_link_send(type, seq, flags, payload, len);
}

static void port_count_drop(void *ctx)
{
    (void)ctx;
    td_link_count_drop();
}

static uint32_t port_now_ms(void *ctx)
{
    (void)ctx;
    return (uint32_t)(esp_timer_get_time() / 1000);
}

static const td_transfer_ports_t s_ports = {
    .alloc = port_alloc,
    .release = port_release,
    .update_cancel = port_update_cancel,
    .asset_abort = port_asset_abort,
    .update_write = port_update_write,
    .asset_write = port_asset_write,
    .origin = port_origin,
    .origin_live = port_origin_live,
    .photo_idle = port_photo_idle,
    .send = port_send,
    .count_drop = port_count_drop,
    .now_ms = port_now_ms,
};

/* The link task is the only user, and a transfer with no kind is all zeros. */
static td_transfer_t s_transfer = { .ports = &s_ports };

td_transfer_t *td_link_transfer(void)
{
    return &s_transfer;
}

void td_link_files_chunk(uint8_t seq, const uint8_t *payload, size_t len)
{
    td_transfer_chunk(&s_transfer, seq, payload, len);
}

void td_link_files_ack(const uint8_t *payload, size_t len)
{
    td_transfer_ack(&s_transfer, payload, len);
}

void td_link_files_service(uint32_t now_ms)
{
    td_transfer_service(&s_transfer, now_ms);
}

void td_link_files_select(void)
{
    td_transfer_select(&s_transfer);
}
