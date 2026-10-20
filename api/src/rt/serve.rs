use std::future::IntoFuture;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::serve::ListenerExt;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::config::env_str;

pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

pub async fn plain(addr: SocketAddr, app: Router, shutdown: CancellationToken) -> io::Result<()> {
    if env_str("TLS_PROXY_PROTOCOL", "false") == "true" {
        warn!("chat-api reads no PROXY header, haproxy must send it plain HTTP");
    }
    let listener = TcpListener::bind(addr).await?;
    drain(listener, app, shutdown, SHUTDOWN_GRACE).await
}

pub async fn drain(
    listener: TcpListener,
    app: Router,
    shutdown: CancellationToken,
    grace: Duration,
) -> io::Result<()> {
    let listener = listener.tap_io(|stream| {
        let _ = stream.set_nodelay(true);
    });
    let serving = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.clone().cancelled_owned())
    .into_future();
    let deadline = async move {
        shutdown.cancelled().await;
        tokio::time::sleep(grace).await;
    };
    tokio::select! {
        served = serving => served,
        () = deadline => {
            warn!(grace_s = grace.as_secs(), "chat-api stopped with requests still in flight");
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
