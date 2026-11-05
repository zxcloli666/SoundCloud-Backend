mod bucket;
mod claims;
pub mod config;
mod decide;
pub mod dto;
pub mod effort;
mod exchange;
pub mod gate;
mod handlers;
mod keys;
mod kind;
mod metrics;
mod prepare;
mod refusal;
pub mod seed;
pub mod signed;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use deadpool_redis::Pool;
use tokio::sync::Semaphore;

use crate::modules::auth::sealed::config::SealedCfg;
use crate::modules::auth::sealed::gate::Gates;
use crate::modules::auth::sealed::seed::Seeds;
use crate::modules::auth::{AuthService, LinkService};
use crate::modules::chat::channel::ock::OckRing;

pub const WORK_PERMITS: usize = 2_048;

pub struct Sealed {
    cfg: SealedCfg,
    ocks: OckRing,
    seeds: Seeds,
    redis: Pool,
    gates: Gates,
    inner: Router,
    busy: Semaphore,
}

impl Sealed {
    pub fn load(
        cfg: SealedCfg,
        redis: Pool,
        auth: Arc<AuthService>,
        link: Arc<LinkService>,
    ) -> anyhow::Result<Arc<Self>> {
        let (ocks, seed_key) = keys::load(&cfg)?;
        Ok(Self::new(cfg, ocks, seed_key, redis, auth, link))
    }

    pub fn new(
        cfg: SealedCfg,
        ocks: OckRing,
        seed_key: [u8; 32],
        redis: Pool,
        auth: Arc<AuthService>,
        link: Arc<LinkService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            seeds: Seeds::new(seed_key, cfg.node.clone()),
            gates: Gates::new(&cfg),
            inner: handlers::inner(auth, link),
            busy: Semaphore::new(WORK_PERMITS),
            cfg,
            ocks,
            redis,
        })
    }
}

pub fn router<S>(sealed: Option<Arc<Sealed>>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    match sealed {
        Some(sealed) => Router::new()
            .route(
                "/auth/x",
                post(exchange::exchange)
                    .layer(DefaultBodyLimit::max(sc_chat_wire::seal::MAX_ENVELOPE)),
            )
            .with_state(sealed),
        None => Router::new(),
    }
}
