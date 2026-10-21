mod config;
mod router;
mod serve;
mod state;

pub use config::{Role, role};

use tracing::info;

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

    if let Some(tls_cfg) = tls_common::TlsConfig::from_env() {
        info!("starting with TLS (ACME)");
        tls_common::serve(tls_cfg, app).await;
    } else {
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.port));
        info!(%addr, "starting plain HTTP");
        serve::plain(addr, app, draining)
            .await
            .expect("Server error");
    }
    info!("chat-api stopped");
}
