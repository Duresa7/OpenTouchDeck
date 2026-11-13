//! The accept loop the integration tests serve the router with: hyper over HTTP/1.1 with the
//! connection and time limits in `crate::Limits`.
use axum::Router;
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::{conn::auto::Builder, graceful::GracefulShutdown},
    service::TowerToHyperService,
};
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
    task::JoinSet,
};

/// Serves `router` until `stop` turns true, then drains for at most `limits.drain`.
pub(super) async fn serve(
    listener: TcpListener,
    router: Router,
    limits: crate::Limits,
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
