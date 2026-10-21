mod config;
mod router;
mod serve;
mod state;

pub use config::{Role, role};

use tracing::{info, warn};

use crate::config::env_str;
use crate::modules::chat;

pub async fn run() {
    let config = config::RtConfig::from_env();
    info!(port = config.port, "chat-api starting");
    let state = state::build(&config).await;
    chat::sweeps::spawn(state.chat.clone());
    tokio::spawn(chat::moderation::task::run(state.chat.clone()));
    let shutdown = state.chat.shutdown.clone();
    let draining = shutdown.clone();
    tokio::spawn(async move {
        tls_common::shutdown_signal().await;
        shutdown.cancel();
    });
    let app = router::build(state);

    if env_str("TLS_ENABLED", "false") == "true" {
        warn!("chat-api serves plain h2c behind haproxy, the TLS settings are ignored");
    }
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.port));
    info!(%addr, "starting plain HTTP");
    serve::plain(addr, app, draining)
        .await
        .expect("Server error");
    info!("chat-api stopped");
}
