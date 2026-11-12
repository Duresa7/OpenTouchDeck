#include "td_netlink_tcp.h"
#include <string.h>

#define WRITE_MS 1000
#define REFUSE_MS 100
#define DRAIN_MS 100

static uint32_t now_ms(const td_netlink_tcp_t *n)
{
    return n->deck.env.clock(n->deck.env.ctx);
}

static bool write_all(td_netlink_tcp_t *n, int fd, const uint8_t *buf, size_t len, uint32_t timeout_ms)
{
    const uint32_t until = now_ms(n) + timeout_ms;
    size_t done = 0;
    while (done < len) {
        const int sent = n->io.write(n->io.ctx, fd, buf + done, len - done);
        if (sent > 0) {
            done += (size_t)sent;
            continue;
        }
        if (sent < 0 && sent != TD_NETLINK_TCP_AGAIN) {
            return false;
        }
        const int32_t left = (int32_t)(until - now_ms(n));
        if (left <= 0) {
            return false;
        }
        n->io.wait_write(n->io.ctx, fd, left < 100 ? (uint32_t)left : 100);
    }
    return true;
}

/* Hands a socket to a closer, with its last item (none for len 0). With every closer in use, the item gets one try
 * and the socket closes at once. */
static void closer_add(td_netlink_tcp_t *n, int fd, const uint8_t *item, size_t len)
{
    len = len > TD_NETLINK_TCP_CLOSER_ITEM_MAX ? 0 : len;
    for (size_t i = 0; i < TD_NETLINK_TCP_CLOSERS; i++) {
        td_netlink_tcp_closer_t *c = &n->closers[i];
        if (c->fd < 0) {
            *c = (td_netlink_tcp_closer_t){ .fd = fd, .len = (uint8_t)len, .until_ms = now_ms(n) + REFUSE_MS };
            if (len) {
                memcpy(c->item, item, len);
            }
            return;
        }
    }
    if (len) {
        n->io.write(n->io.ctx, fd, item, len);
    }
    n->io.close(n->io.ctx, fd);
}

/* One step of each closer, never waiting. Do not half-close before draining: a peer FIN racing lwIP
 * shutdown used to double-free its receive buffers (d268ada). Ownership ends with exactly one close. */
static void closers_step(td_netlink_tcp_t *n)
{
    const uint32_t now = now_ms(n);
    for (size_t i = 0; i < TD_NETLINK_TCP_CLOSERS; i++) {
        td_netlink_tcp_closer_t *c = &n->closers[i];
        if (c->fd < 0) {
            continue;
        }
        if (!c->draining) {
            if (c->sent < c->len) {
                const int sent = n->io.write(n->io.ctx, c->fd, c->item + c->sent, c->len - c->sent);
                if (sent > 0) {
                    c->sent += (uint8_t)sent;
                } else if (sent < 0 && sent != TD_NETLINK_TCP_AGAIN) {
                    c->sent = c->len;
                }
            }
            if (c->sent < c->len && (int32_t)(now - c->until_ms) < 0) {
                continue;
            }
            c->draining = true;
            c->until_ms = now + DRAIN_MS;
        }
        uint8_t sink[64];
        int got;
        while ((got = n->io.read(n->io.ctx, c->fd, sink, sizeof sink)) > 0) {
        }
        const bool open = got == TD_NETLINK_TCP_AGAIN;
        if (!open || (int32_t)(now - c->until_ms) >= 0) {
            n->io.close(n->io.ctx, c->fd);
            *c = (td_netlink_tcp_closer_t){ .fd = -1 };
        }
    }
}

static bool closers_busy(const td_netlink_tcp_t *n)
{
    for (size_t i = 0; i < TD_NETLINK_TCP_CLOSERS; i++) {
        if (n->closers[i].fd >= 0) {
            return true;
        }
    }
    return false;
}

static void reader_reset(td_netlink_tcp_reader_t *r)
{
    r->in_body = false;
    r->want = 0;
    r->got = 0;
}

/* ---- The owner ---- */

static void session_ended(td_netlink_tcp_t *n)
{
    if (n->ready) {
        n->ready = false;
        n->io.session(n->io.ctx, false);
    }
}

/* The owner's connection is gone: how says whether the peer or the Device ended it. An orderly close hands the socket
 * to a closer with its last item, if any. */
static void drop_owner_with(td_netlink_tcp_t *n, bool orderly, const uint8_t *item, size_t len, td_netlink_gone_t how)
{
    if (n->owner.fd < 0) {
        return;
    }
    if (orderly) {
        closer_add(n, n->owner.fd, item, len);
    } else {
        n->io.close(n->io.ctx, n->owner.fd);
    }
    n->owner.fd = -1;
    reader_reset(&n->owner);
    td_netlink_conn_close(&n->conn, &n->deck, how);
    td_netlink_wipe(n->body, sizeof n->body);
    session_ended(n);
}

static void drop_owner(td_netlink_tcp_t *n, bool orderly, td_netlink_gone_t how)
{
    drop_owner_with(n, orderly, NULL, 0, how);
}

static void act(td_netlink_tcp_t *n, td_netlink_action_t action)
{
    switch (action) {
    case TD_NETLINK_NOTHING:
        break;
    case TD_NETLINK_SEND:
        if (!write_all(n, n->owner.fd, n->item.item, n->item.len, WRITE_MS)) {
            drop_owner(n, false, TD_NETLINK_GONE_PEER);
        } else if (n->conn.state == TD_NETLINK_CONN_READY && !n->ready) {
            n->ready = true;
            n->io.session(n->io.ctx, true);
        }
        break;
    case TD_NETLINK_SEND_CLOSE:
        if (n->item.len <= TD_NETLINK_TCP_CLOSER_ITEM_MAX) {
            /* An error item: the closer writes it. */
            drop_owner_with(n, true, n->item.item, n->item.len, TD_NETLINK_GONE_DEVICE);
        } else {
            write_all(n, n->owner.fd, n->item.item, n->item.len, WRITE_MS);
            drop_owner(n, true, TD_NETLINK_GONE_DEVICE);
        }
        break;
    case TD_NETLINK_CLOSE:
        drop_owner(n, false, TD_NETLINK_GONE_DEVICE);
        break;
    }
    td_netlink_wipe(&n->item, sizeof n->item);
}

static void become_owner(td_netlink_tcp_t *n, int fd, const uint8_t *peer, uint8_t peer_len)
{
    n->owner.fd = fd;
    memcpy(n->owner.peer, peer, peer_len);
    n->owner.peer_len = peer_len;
    n->owner.started_ms = now_ms(n);
    reader_reset(&n->owner);
    td_netlink_conn_init(&n->conn, &n->deck, peer, peer_len);
}

/* Reads what the owner's socket holds, one length prefix and one body at a time. A bad length closes before any
 * body is read. */
static void read_owner(td_netlink_tcp_t *n)
{
    td_netlink_tcp_reader_t *r = &n->owner;
    while (r->fd >= 0) {
        uint8_t *dst = r->in_body ? n->body + r->got : r->head + r->got;
        const size_t want = (r->in_body ? r->want : sizeof r->head) - r->got;
        const int got = n->io.read(n->io.ctx, r->fd, dst, want);
        if (got == TD_NETLINK_TCP_AGAIN) {
            return;
        }
        if (got <= 0) {
            drop_owner(n, false, TD_NETLINK_GONE_PEER);
            return;
        }
        if (!r->in_body && r->got == 0) {
            td_netlink_conn_item_started(&n->conn, &n->deck);
        }
        r->got += (size_t)got;
        if (!r->in_body) {
            if (r->got < sizeof r->head) {
                continue;
            }
            const uint16_t len = (uint16_t)(r->head[0] << 8 | r->head[1]);
            uint16_t min, max;
            td_netlink_conn_limits(&n->conn, &min, &max);
            r->got = 0;
            if (len < min || len > max) {
                act(n, td_netlink_conn_bad_length(&n->conn, &n->deck, &n->item));
                return;
            }
            r->in_body = true;
            r->want = len;
            continue;
        }
        if (r->got < r->want) {
            continue;
        }
        const size_t len = r->want;
        reader_reset(r);
        if (n->conn.state != TD_NETLINK_CONN_READY) {
            act(n, td_netlink_conn_item(&n->conn, &n->deck, n->body, len, &n->item));
        } else if (!td_netlink_conn_record(&n->conn, &n->deck, n->body, len, n->plain)) {
            drop_owner(n, false, TD_NETLINK_GONE_DEVICE);
        } else {
            n->io.receive(n->io.ctx, n->plain, len - TD_NETLINK_TAG_LEN);
            td_netlink_wipe(n->plain, sizeof n->plain);
        }
    }
}

/* ---- Connections waiting to take over ---- */

static const td_netlink_conn_t *owner_conn(td_netlink_tcp_t *n)
{
    return n->owner.fd >= 0 ? &n->conn : NULL;
}

/* Hands the waiter's socket to a closer with its last item, or with none. The core slot is already free. */
static void waiter_close(td_netlink_tcp_t *n, size_t i, const uint8_t *item, size_t len)
{
    closer_add(n, n->waiters[i].fd, item, len);
    n->waiters[i].fd = -1;
    reader_reset(&n->waiters[i]);
    td_netlink_wipe(n->waiter_body[i], sizeof n->waiter_body[i]);
}

/* A waiter's whole first item: it takes over, becomes the owner of a Device without one, or is refused. */
static void waiter_item(td_netlink_tcp_t *n, size_t i, size_t len)
{
    const td_netlink_action_t action =
        td_netlink_waiter_item(&n->deck, owner_conn(n), (int)i, n->waiter_body[i], len, &n->item);
    if (action == TD_NETLINK_NOTHING) {
        const td_netlink_tcp_reader_t waiter = n->waiters[i];
        n->waiters[i].fd = -1;
        reader_reset(&n->waiters[i]);
        if (n->owner.fd >= 0) {
            td_netlink_conn_end(&n->conn, &n->deck, TD_NETLINK_END_TAKEOVER);
            drop_owner(n, false, TD_NETLINK_GONE_DEVICE);
        }
        become_owner(n, waiter.fd, waiter.peer, waiter.peer_len);
        td_netlink_conn_item_started(&n->conn, &n->deck);
        act(n, td_netlink_conn_item(&n->conn, &n->deck, n->waiter_body[i], len, &n->item));
    } else {
        waiter_close(n, i, n->item.item, action == TD_NETLINK_SEND_CLOSE ? n->item.len : 0);
    }
    td_netlink_wipe(n->waiter_body[i], sizeof n->waiter_body[i]);
    td_netlink_wipe(&n->item, sizeof n->item);
}

static void read_waiter(td_netlink_tcp_t *n, size_t i)
{
    td_netlink_tcp_reader_t *r = &n->waiters[i];
    while (r->fd >= 0) {
        uint8_t *dst = r->in_body ? n->waiter_body[i] + r->got : r->head + r->got;
        const size_t want = (r->in_body ? r->want : sizeof r->head) - r->got;
        const int got = n->io.read(n->io.ctx, r->fd, dst, want);
        if (got == TD_NETLINK_TCP_AGAIN) {
            return;
        }
        if (got <= 0) {
            /* A FIN or RST before its first item counts against the peer. */
            td_netlink_waiter_gone(&n->deck, (int)i);
            n->io.close(n->io.ctx, r->fd);
            r->fd = -1;
            reader_reset(r);
            return;
        }
        if (!r->in_body && r->got == 0) {
            td_netlink_waiter_started(&n->deck, (int)i);
        }
        r->got += (size_t)got;
        if (!r->in_body) {
            if (r->got < sizeof r->head) {
                continue;
            }
            const uint16_t len = (uint16_t)(r->head[0] << 8 | r->head[1]);
            if (len < TD_NETLINK_HANDSHAKE_MIN || len > TD_NETLINK_HANDSHAKE_MAX) {
                /* Not a session.open that could take over: busy, like any other first item. */
                td_netlink_waiter_drop(&n->deck, (int)i);
                waiter_close(n, i, td_netlink_busy_item, sizeof td_netlink_busy_item);
                return;
            }
            r->got = 0;
            r->in_body = true;
            r->want = len;
            continue;
        }
        if (r->got == r->want) {
            waiter_item(n, i, r->want);
            return;
        }
    }
}

static void tick_waiters(td_netlink_tcp_t *n)
{
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        if (n->waiters[i].fd >= 0
            && td_netlink_waiter_tick(&n->deck, owner_conn(n), (int)i, &n->item) == TD_NETLINK_SEND_CLOSE) {
            waiter_close(n, i, n->item.item, n->item.len);
            td_netlink_wipe(&n->item, sizeof n->item);
        }
    }
}

static void drop_waiters(td_netlink_tcp_t *n)
{
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        if (n->waiters[i].fd >= 0) {
            td_netlink_waiter_drop(&n->deck, (int)i);
            n->io.close(n->io.ctx, n->waiters[i].fd);
            n->waiters[i].fd = -1;
            reader_reset(&n->waiters[i]);
        }
    }
}

void td_netlink_tcp_accept(td_netlink_tcp_t *n, int fd, const uint8_t *peer, uint8_t peer_len)
{
    int slot, evicted;
    const td_netlink_admit_t admit = td_netlink_admit(&n->deck, owner_conn(n), peer, peer_len, &slot, &evicted);
    if (evicted >= 0) {
        /* The core reused its slot for this connection: the waiter it pushed out gets busy. */
        waiter_close(n, (size_t)evicted, td_netlink_busy_item, sizeof td_netlink_busy_item);
    }
    switch (admit) {
    case TD_NETLINK_ADMIT_OWNER:
        become_owner(n, fd, peer, peer_len);
        break;
    case TD_NETLINK_ADMIT_WAIT: {
        td_netlink_tcp_reader_t *w = &n->waiters[slot];
        w->fd = fd;
        memcpy(w->peer, peer, peer_len);
        w->peer_len = peer_len;
        w->started_ms = now_ms(n);
        reader_reset(w);
        break;
    }
    case TD_NETLINK_ADMIT_BUSY:
        closer_add(n, fd, td_netlink_busy_item, sizeof td_netlink_busy_item);
        break;
    }
}

void td_netlink_tcp_init(td_netlink_tcp_t *n, const td_netlink_tcp_io_t *io)
{
    n->io = *io;
    n->owner.fd = -1;
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        n->waiters[i].fd = -1;
    }
    for (size_t i = 0; i < TD_NETLINK_TCP_CLOSERS; i++) {
        n->closers[i].fd = -1;
    }
}

void td_netlink_tcp_read(td_netlink_tcp_t *n, int fd)
{
    if (fd < 0) {
        return;
    }
    if (n->owner.fd == fd) {
        read_owner(n);
        return;
    }
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        if (n->waiters[i].fd == fd) {
            read_waiter(n, i);
            return;
        }
    }
}

void td_netlink_tcp_tick(td_netlink_tcp_t *n)
{
    td_netlink_deck_tick(&n->deck);
    if (n->owner.fd >= 0) {
        act(n, td_netlink_conn_tick(&n->conn, &n->deck, &n->item));
    }
    tick_waiters(n);
    closers_step(n);
}

bool td_netlink_tcp_busy(const td_netlink_tcp_t *n)
{
    if (n->owner.fd >= 0 || closers_busy(n)) {
        return true;
    }
    for (size_t i = 0; i < TD_NETLINK_WAITERS; i++) {
        if (n->waiters[i].fd >= 0) {
            return true;
        }
    }
    return false;
}

void td_netlink_tcp_send(td_netlink_tcp_t *n, const uint8_t *body, size_t len)
{
    if (n->owner.fd < 0 || n->conn.state != TD_NETLINK_CONN_READY) {
        return;
    }
    if (!td_netlink_conn_send(&n->conn, body, len, n->wire)
        || !write_all(n, n->owner.fd, n->wire, 2 + len + TD_NETLINK_TAG_LEN, WRITE_MS)) {
        drop_owner(n, false, TD_NETLINK_GONE_DEVICE);
    }
}

void td_netlink_tcp_flushed(td_netlink_tcp_t *n)
{
    if (n->owner.fd >= 0 && td_netlink_conn_worn(&n->conn)) {
        td_netlink_conn_end(&n->conn, &n->deck, TD_NETLINK_END_LOCAL);
        drop_owner(n, true, TD_NETLINK_GONE_DEVICE);
    }
}

void td_netlink_tcp_end(td_netlink_tcp_t *n, const uint8_t *companion_id)
{
    const bool session = n->conn.state == TD_NETLINK_CONN_SESSION_PROOF || n->conn.state == TD_NETLINK_CONN_READY;
    if (n->owner.fd >= 0
        && (!companion_id || (session && !memcmp(n->conn.companion_id, companion_id, TD_NETLINK_COMPANION_ID_LEN)))) {
        td_netlink_conn_end(&n->conn, &n->deck, TD_NETLINK_END_LOCAL);
        drop_owner(n, false, TD_NETLINK_GONE_DEVICE);
    }
    if (!companion_id) {
        drop_waiters(n);
    }
}
