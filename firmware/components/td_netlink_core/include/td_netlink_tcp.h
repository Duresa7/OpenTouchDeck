#pragma once
#include "td_netlink.h"

#define TD_NETLINK_TCP_CLOSERS 4
#define TD_NETLINK_TCP_CLOSER_ITEM_MAX 8
/* Socket callbacks are nonblocking: positive byte count, 0 for EOF, -1 for fatal error, -2 for would-block.
 * wait_write waits at most timeout_ms. No shutdown/half-close operation is used. Callbacks run on the net task.
 * The clock is the deck environment's monotonic millisecond clock. */
#define TD_NETLINK_TCP_AGAIN (-2)
typedef struct {
    int (*read)(void *ctx, int fd, uint8_t *buf, size_t len);
    int (*write)(void *ctx, int fd, const uint8_t *buf, size_t len);
    void (*close)(void *ctx, int fd);
    void (*wait_write)(void *ctx, int fd, uint32_t timeout_ms);
    void (*session)(void *ctx, bool ready);
    void (*receive)(void *ctx, const uint8_t *body, size_t len);
    void *ctx;
} td_netlink_tcp_io_t;

/* One TCP connection's item reader. */
typedef struct {
    int fd;
    uint8_t peer[TD_NETLINK_PEER_LEN];
    uint8_t peer_len;
    uint32_t started_ms;
    uint8_t head[2];
    bool in_body;
    size_t want;
    size_t got;
} td_netlink_tcp_reader_t;

/* A socket the Device is closing: it writes its last item within 100 ms and reads whatever still arrives for
 * 100 ms, so the peer reads that item instead of losing it to a reset. */
typedef struct {
    int fd;
    uint8_t item[TD_NETLINK_TCP_CLOSER_ITEM_MAX];
    uint8_t len;
    uint8_t sent;
    bool draining;
    uint32_t until_ms;
} td_netlink_tcp_closer_t;

typedef struct {
    td_netlink_deck_t deck;
    td_netlink_conn_t conn;
    td_netlink_tcp_reader_t owner;
    /* Connections waiting for their first item while the owner is unproven, each in its deck.waiters slot: a
     * session.open with a stored Pairing's open_tag takes over. */
    td_netlink_tcp_reader_t waiters[TD_NETLINK_WAITERS];
    uint8_t waiter_body[TD_NETLINK_WAITERS][TD_NETLINK_HANDSHAKE_MAX];
    td_netlink_tcp_closer_t closers[TD_NETLINK_TCP_CLOSERS];
    td_netlink_out_t item;
    uint8_t body[TD_NETLINK_SEALED_MAX];
    uint8_t plain[TD_NETLINK_PLAIN_MAX];
    uint8_t wire[TD_NETLINK_RECORD_WIRE_MAX];
    td_netlink_tcp_io_t io;
    bool ready;
} td_netlink_tcp_t;

/* Zero-initialize state and initialize deck before use. State is caller-allocated (PSRAM on the Device).
 * Fields are read-only to adapters, except deck management and conn_hello. Every accepted fd is transferred exactly once, including refusals. */
void td_netlink_tcp_init(td_netlink_tcp_t *n, const td_netlink_tcp_io_t *io);
void td_netlink_tcp_accept(td_netlink_tcp_t *n, int fd, const uint8_t *peer, uint8_t peer_len);
/* Feed readable descriptors from select, then tick once per poll (including when select times out). */
void td_netlink_tcp_read(td_netlink_tcp_t *n, int fd);
void td_netlink_tcp_tick(td_netlink_tcp_t *n);
bool td_netlink_tcp_busy(const td_netlink_tcp_t *n);
void td_netlink_tcp_send(td_netlink_tcp_t *n, const uint8_t *body, size_t len);
/* Called after the outgoing queue is empty, so record-limit closing stays between requests. */
void td_netlink_tcp_flushed(td_netlink_tcp_t *n);
void td_netlink_tcp_end(td_netlink_tcp_t *n, const uint8_t *companion_id);
