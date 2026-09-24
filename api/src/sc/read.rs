use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tracing::debug;

use crate::error::{AppError, AppResult};
use crate::modules::auth::{TokenKind, TokenProvider, try_with_chain};
use crate::sc::{EGRESS_APP, FetchStrategy, PgEgressHealth, hedge, race, within_budget};
use sc_transport::{Apiv2Proxy, EGRESS_RELAY_LUA, EgressHealth, ScClient};

const HEDGE_DELAY: Duration = Duration::from_millis(700);
const CALL_BUDGET: Duration = Duration::from_secs(20);

pub struct ScReadService {
    sc: ScClient,
    proxy: Apiv2Proxy,
    tokens: Arc<TokenProvider>,
    lua_health: EgressHealth,
    strategy: FetchStrategy,
}

impl ScReadService {
    pub fn new(sc: ScClient, tokens: Arc<TokenProvider>, pg: sqlx::PgPool) -> Arc<Self> {
        let proxy = Apiv2Proxy::new(sc.clone());
        Arc::new(Self {
            sc,
            proxy,
            tokens,
            lua_health: EgressHealth::new(
                EGRESS_RELAY_LUA,
                EGRESS_APP,
                Some(PgEgressHealth::new(pg)),
            ),
            strategy: FetchStrategy::from_env(),
        })
    }

    pub async fn resolve(&self, kind: TokenKind, url: &str) -> AppResult<Value> {
        self.run(
            Self::timed("relay_lua", "resolve", self.resolve_lua(url)),
            Self::timed("backup", "resolve", self.resolve_chain(kind, url)),
        )
        .await
    }

    async fn timed<T>(
        tier: &'static str,
        operation: &'static str,
        fut: impl std::future::Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        let started = std::time::Instant::now();
        let result = Box::pin(fut).await;
        let outcome = match &result {
            Ok(_) => crate::metrics::Outcome::Ok,
            Err(AppError::ScUnreachable(_)) => crate::metrics::Outcome::Miss,
            Err(AppError::SoundCloudRefreshTimedOut) => crate::metrics::Outcome::Timeout,
            Err(_) => crate::metrics::Outcome::Error,
        };
        if let Err(error) = &result {
            crate::metrics::record_sc_failure(error);
        }
        crate::metrics::record_sc_tier(tier, operation, outcome, started.elapsed());
        result
    }

    pub async fn track_by_id(&self, kind: TokenKind, sc_track_id: &str) -> AppResult<Value> {
        self.run(
            Self::timed(
                "relay_lua",
                "track",
                self.entity_lua(self.sc.track_by_id_via_relay(sc_track_id)),
            ),
            Self::timed("backup", "track", self.track_chain(kind, sc_track_id)),
        )
        .await
    }

    pub async fn user_by_id(&self, kind: TokenKind, user_id: &str) -> AppResult<Value> {
        self.run(
            Self::timed(
                "relay_lua",
                "user",
                self.entity_lua(self.sc.user_by_id_via_relay(user_id)),
            ),
            Self::timed("backup", "user", self.user_chain(kind, user_id)),
        )
        .await
    }

    pub async fn playlist_meta(&self, kind: TokenKind, playlist_id: &str) -> AppResult<Value> {
        self.run(
            Self::timed(
                "relay_lua",
                "playlist_meta",
                self.entity_lua(self.sc.playlist_full_via_relay(playlist_id, false)),
            ),
            Self::timed(
                "backup",
                "playlist_meta",
                self.playlist_meta_chain(kind, playlist_id),
            ),
        )
        .await
    }

    async fn run<T>(
        &self,
        lua: impl std::future::Future<Output = AppResult<T>>,
        backup: impl std::future::Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        within_budget(CALL_BUDGET, Box::pin(self.walk_tiers(lua, backup))).await
    }

    async fn walk_tiers<T>(
        &self,
        lua: impl std::future::Future<Output = AppResult<T>>,
        backup: impl std::future::Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        if !self.sc.has_relay() || self.lua_health.is_open().await {
            return backup.await;
        }
        match self.strategy {
            FetchStrategy::Fallback => match lua.await {
                Ok(v) => Ok(v),
                Err(_) => backup.await,
            },
            FetchStrategy::Hedge => hedge(lua, HEDGE_DELAY, backup).await,
            FetchStrategy::Race => race(lua, backup).await,
        }
    }

    async fn entity_lua(
        &self,
        fut: impl std::future::Future<Output = sc_transport::RelayRead<Value>>,
    ) -> AppResult<Value> {
        let read = fut.await;
        crate::metrics::set_relay_breaker_open(self.lua_health.observe(&read).await);
        match read {
            sc_transport::RelayRead::Found(mut v) => {
                sc_transport::normalize_v2_to_v1(&mut v);
                Ok(v)
            }
            sc_transport::RelayRead::Missing => {
                Err(AppError::ScUnreachable("relay: entity not found".into()))
            }
            sc_transport::RelayRead::Unavailable => {
                Err(AppError::ScUnreachable("relay: no result".into()))
            }
        }
    }

    async fn resolve_lua(&self, url: &str) -> AppResult<Value> {
        self.entity_lua(self.sc.resolve_track_via_relay(url)).await
    }

    async fn resolve_chain(&self, kind: TokenKind, url: &str) -> AppResult<Value> {
        match self.proxy.resolve(url).await {
            Ok(mut v) => {
                sc_transport::normalize_v2_to_v1(&mut v);
                Ok(v)
            }
            Err(e) => {
                debug!(error = %e, "[read] apiv2-proxy resolve failed, apiv1 fallback");
                let params = [("url".to_string(), url.to_string())];
                self.apiv1_get(kind, "/resolve", Some(&params)).await
            }
        }
    }

    async fn track_chain(&self, kind: TokenKind, id: &str) -> AppResult<Value> {
        match self.proxy.track(id).await {
            Ok(mut v) => {
                sc_transport::normalize_v2_to_v1(&mut v);
                Ok(v)
            }
            Err(_) => self.apiv1_get(kind, &format!("/tracks/{id}"), None).await,
        }
    }

    async fn user_chain(&self, kind: TokenKind, id: &str) -> AppResult<Value> {
        match self.proxy.user(id).await {
            Ok(mut v) => {
                sc_transport::normalize_v2_to_v1(&mut v);
                Ok(v)
            }
            Err(_) => self.apiv1_get(kind, &format!("/users/{id}"), None).await,
        }
    }

    async fn playlist_meta_chain(&self, kind: TokenKind, id: &str) -> AppResult<Value> {
        match self.proxy.playlist(id, false).await {
            Ok(v) => Ok(v),
            Err(_) => {
                self.apiv1_get(kind, &format!("/playlists/{id}"), None)
                    .await
            }
        }
    }

    async fn apiv1_get(
        &self,
        kind: TokenKind,
        path: &str,
        params: Option<&[(String, String)]>,
    ) -> AppResult<Value> {
        let chain = self.tokens.chain(kind).await?;
        try_with_chain(&chain, |tok| {
            let sc = self.sc.clone();
            let path = path.to_string();
            let params = params.map(<[_]>::to_vec);
            async move { sc.api_get_value(&path, &tok, params.as_deref()).await }
        })
        .await
    }
}
