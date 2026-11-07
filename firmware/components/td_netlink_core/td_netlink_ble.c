#include "td_netlink_ble.h"
#include "td_netlink.h"
#include <string.h>

void td_netlink_reasm_init(td_netlink_reasm_t *r, uint16_t max_fragment, uint8_t *buf, size_t cap)
{
    memset(r, 0, sizeof *r);
    r->max_fragment = max_fragment;
    r->buf = buf;
    r->cap = cap;
}

bool td_netlink_reasm_records(td_netlink_reasm_t *r)
{
    if (r->pending) {
        r->closed = true;
        return false;
    }
    r->records = true;
    return true;
}

static td_netlink_frag_t close_reasm(td_netlink_reasm_t *r)
{
    r->closed = true;
    r->pending = false;
    return TD_NETLINK_FRAG_CLOSE;
}

td_netlink_frag_t td_netlink_reasm_push(td_netlink_reasm_t *r, const uint8_t *chunk, size_t len, const uint8_t **item,
                                        size_t *item_len)
{
    if (r->closed || len < 1 || len > r->max_fragment) {
        return close_reasm(r);
    }
    if (!r->pending) {
        if (len < 2) {
            return close_reasm(r);
        }
        const uint16_t body = (uint16_t)(chunk[0] << 8 | chunk[1]);
        const uint16_t min = r->records ? TD_NETLINK_SEALED_MIN : TD_NETLINK_HANDSHAKE_MIN;
        const uint16_t max = r->records ? TD_NETLINK_SEALED_MAX : TD_NETLINK_HANDSHAKE_MAX;
        if (body < min || body > max || (size_t)body + 2 > r->cap) {
            return close_reasm(r);
        }
        r->pending = true;
        r->need = (uint16_t)(body + 2);
        r->have = 0;
    }
    if ((size_t)r->have + len > r->need) {
        return close_reasm(r);
    }
    const bool last = r->have + len == r->need;
    if (last) {
        if (r->f && len > r->f) {
            return close_reasm(r);
        }
    } else {
        if (len < TD_NETLINK_FRAGMENT_MIDDLE_MIN || (r->f && len != r->f)) {
            return close_reasm(r);
        }
        r->f = (uint16_t)len;
    }
    memcpy(r->buf + r->have, chunk, len);
    r->have = (uint16_t)(r->have + len);
    if (!last) {
        return TD_NETLINK_FRAG_MORE;
    }
    r->pending = false;
    *item = r->buf;
    *item_len = r->need;
    return TD_NETLINK_FRAG_ITEM;
}

static void u64be(uint64_t v, uint8_t out[8])
{
    for (int i = 7; i >= 0; i--) {
        out[i] = (uint8_t)v;
        v >>= 8;
    }
}

void td_netlink_credit_init(td_netlink_credit_t *c, uint32_t initial)
{
    memset(c, 0, sizeof *c);
    c->initial = initial < TD_NETLINK_CREDIT_MIN ? TD_NETLINK_CREDIT_MIN : initial;
}

bool td_netlink_credit_enable(td_netlink_credit_t *c, td_netlink_notify_t which, uint8_t value[8])
{
    if (c->closed) {
        return false;
    }
    if (which == TD_NETLINK_NOTIFY_TX) {
        c->tx = true;
    } else {
        c->flow = true;
    }
    if (c->started || !c->tx || !c->flow) {
        return false;
    }
    c->started = true;
    c->limit = c->initial;
    u64be(c->limit, value);
    return true;
}

void td_netlink_credit_disable(td_netlink_credit_t *c)
{
    c->closed = true;
}

bool td_netlink_credit_write(td_netlink_credit_t *c, size_t len)
{
    if (c->closed || !c->started || c->written + len > c->limit) {
        c->closed = true;
        return false;
    }
    c->written += len;
    return true;
}

bool td_netlink_credit_grant(td_netlink_credit_t *c, uint64_t limit, uint8_t value[8])
{
    if (c->closed || !c->started || limit < c->limit) {
        c->closed = true;
        return false;
    }
    c->limit = limit;
    u64be(limit, value);
    return true;
}
