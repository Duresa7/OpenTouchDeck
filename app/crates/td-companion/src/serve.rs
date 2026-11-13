//! The accept loop: hyper over HTTP/1.1 with the connection and time limits from the contract.
use axum::Router;
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::{conn::auto::Builder, graceful::GracefulShutdown},
    service::TowerToHyperService,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
    task::JoinSet,
};

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

/// Serves `router` until `stop` turns true, then drains for at most `limits.drain`.
pub(crate) async fn serve(
    listener: TcpListener,
    router: Router,
    limits: Limits,
    mut stop: watch::Receiver<bool>,
) {
    let slots = Arc::new(Semaphore::new(limits.connections));
    let graceful = GracefulShutdown::new();
    let mut connections = JoinSet::new();
    let mut builder = Builder::new(TokioExecutor::new()).http1_only();
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read);
    loop {
        while connections.try_join_next().is_some() {}
        let slot = tokio::select! {
            slot = slots.clone().acquire_owned() => slot.expect("the semaphore is never closed"),
            _ = stop.wait_for(|s| *s) => break,
        };
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(_) => continue,
            },
            _ = stop.wait_for(|s| *s) => break,
        };
        let service = TowerToHyperService::new(router.clone());
        let connection = builder
            .serve_connection_with_upgrades(TokioIo::new(stream), service)
            .into_owned();
        let connection = graceful.watch(connection);
        connections.spawn(async move {
            let _ = connection.await;
            drop(slot);
        });
    }
    drop(listener);
    let _ = tokio::time::timeout(limits.drain, graceful.shutdown()).await;
    connections.abort_all();
}

pub(crate) async fn bind(root: &std::path::Path) -> std::io::Result<tokio::net::TcpListener> {
    let settings = root.join("companion-settings.json");
    let port = std::fs::read(&settings)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["port"].as_u64())
        .filter(|p| (49152..=65535).contains(p));
    if let Some(port) = port
        && let Ok(listener) =
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port as u16)).await
    {
        return Ok(listener);
    }
    for _ in 0..128 {
        let mut bytes = [0; 2];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        let port = 49152 + (u16::from_le_bytes(bytes) % 16384);
        if let Ok(listener) =
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await
        {
            std::fs::write(
                settings,
                serde_json::to_vec(&serde_json::json!({"port":port}))?,
            )?;
            return Ok(listener);
        }
    }
    Err(std::io::Error::other("No free Companion port."))
}
