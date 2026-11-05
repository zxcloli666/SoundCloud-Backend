mod config;
mod health;
mod rooms;
mod router;
mod serve;
mod state;
#[cfg(test)]
mod surface_tests;
#[cfg(test)]
mod test_support;

pub use config::{Role, role};

use anyhow::Context;
use tracing::{info, warn};

use crate::config::env_str;
use crate::modules::chat;

pub async fn run() -> anyhow::Result<()> {
    let config = config::RtConfig::from_env();
    info!(port = config.port, "chat-api starting");
    let state = state::build(&config).await;
    let rooms_proxy = match config.rooms.mode {
        rooms::Mode::Proxy => config.rooms.api.as_ref().map(rooms::Proxy::new),
        rooms::Mode::Local => None,
    };
    if let Some(api) = config.rooms.handoff_api() {
        let moved = rooms::hand_off(api, &config.rooms.token, &state.rooms)
            .await
            .context("the rooms handoff must finish before chat-api listens")?;
        info!(moved, "rooms handed off from the api");
    }
    chat::sync::incarnation::current(&state.chat)
        .await
        .expect("chat.meta must hold the incarnation");
    chat::sweeps::spawn(state.chat.clone());
    tokio::spawn(chat::moderation::task::run(state.chat.clone()));
    let shutdown = state.chat.shutdown.clone();
    let draining = shutdown.clone();
    let rooms = state.rooms.clone();
    tokio::spawn(async move {
        tls_common::shutdown_signal().await;
        info!("chat-api drains: held polls answer now, requests in flight finish");
        shutdown.cancel();
        rooms.close();
    });
    let app = router::build(state, rooms_proxy);

    if env_str("TLS_ENABLED", "false") == "true" {
        warn!("chat-api serves plain h2c behind haproxy, the TLS settings are ignored");
    }
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.port));
    info!(%addr, "starting plain HTTP");
    serve::plain(addr, app, draining)
        .await
        .expect("Server error");
    info!("chat-api stopped");
    Ok(())
}
