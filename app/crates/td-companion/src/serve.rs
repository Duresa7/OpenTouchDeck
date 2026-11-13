//! Connection and time limits for the local web adapter, which only the integration tests serve.
use std::time::Duration;

/// Connection and time limits. Tests shrink them; the Companion uses the defaults.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Open connections. The accept loop waits for a free slot.
    pub connections: usize,
    /// Time to receive a request's headers.
    pub header_read: Duration,
    /// Time to receive a request's body.
    pub body_read: Duration,
    /// Time for a whole request, except the WebSocket upgrade.
    pub request: Duration,
    /// How long shutdown waits for open requests before dropping their connections.
    pub drain: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: 128,
            header_read: Duration::from_secs(5),
            body_read: Duration::from_secs(10),
            request: Duration::from_secs(30),
            drain: Duration::from_secs(5),
        }
    }
}
