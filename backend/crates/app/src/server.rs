// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! HTTP/1.1 accept loop. It exists because `axum::serve` offers no way to bound how long a client
//! may take to send its request headers, so a handful of sockets that never finish a request
//! would stay open forever. Caddy is HTTP/1.1 towards the backend, so HTTP/2 is not enabled.

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tower::Service;

#[derive(Debug, Clone, Copy)]
pub struct ServerOptions {
    /// How long a client may take to send the complete request line and headers.
    pub header_read_timeout: Duration,
    /// How long in-flight connections get to finish after shutdown was requested.
    pub shutdown_grace: Duration,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            header_read_timeout: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(10),
        }
    }
}

/// Serves `app` on `listener` until `shutdown` resolves, then stops accepting, lets in-flight
/// requests finish for up to `shutdown_grace` and returns. Long-lived streams (SSE) are not
/// waited for beyond the grace period.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    options: ServerOptions,
    shutdown: impl Future<Output = ()>,
) -> std::io::Result<()> {
    let mut make_service = app.into_make_service_with_connect_info::<SocketAddr>();
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);

    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(connection) => connection,
                Err(error) => {
                    // Typically EMFILE under load: back off instead of spinning.
                    tracing::warn!(%error, "failed to accept a connection");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };

        let tower_service = match make_service.call(peer).await {
            Ok(service) => service,
            Err(infallible) => match infallible {},
        };

        let mut builder = http1::Builder::new();
        builder
            .timer(TokioTimer::new())
            .header_read_timeout(options.header_read_timeout);
        // No connection upgrades are needed: the API only speaks plain request/response and SSE.
        let connection = builder.serve_connection(
            TokioIo::new(stream),
            TowerToHyperService::new(tower_service),
        );
        let connection = graceful.watch(connection);

        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "connection ended with an error");
            }
        });
    }

    drop(listener);
    if tokio::time::timeout(options.shutdown_grace, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("shutdown grace period elapsed with connections still open");
    }
    Ok(())
}

/// Resolves on Ctrl-C or, on Unix, SIGTERM (what `docker stop` sends).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "could not install the SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
