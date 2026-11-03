use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use backend_contracts::auth::AppToken;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::modules::auth::CoreRpc;
use crate::sc::{OAuthCredentials, ScClient};

const DUE_WITHIN: chrono::Duration = chrono::Duration::minutes(10);
const PROBE_EVERY: Duration = Duration::from_secs(30);
const CORE_DOWN_FOR: Duration = Duration::from_secs(5 * 60);
const ISSUE_LOCK_SECONDS: u64 = 60;

pub struct AppTokenFallback {
    pg: PgPool,
    redis: deadpool_redis::Pool,
    sc: ScClient,
    core: Arc<CoreRpc>,
    clock: Mutex<Clock>,
    running: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Clock {
    probed_at: Option<Instant>,
    core_down_since: Option<Instant>,
}

struct DueApp {
    id: Uuid,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

impl AppTokenFallback {
    pub fn new(
        pg: PgPool,
        redis: deadpool_redis::Pool,
        sc: ScClient,
        core: Arc<CoreRpc>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            redis,
            sc,
            core,
            clock: Mutex::new(Clock::default()),
            running: tokio::sync::Mutex::new(()),
        })
    }

    pub async fn top_up(&self) -> bool {
        let Ok(_running) = self.running.try_lock() else {
            return false;
        };
        if !self.probe_due() {
            return false;
        }
        let due = match self.due(None).await {
            Ok(due) => due,
            Err(error) => {
                warn!(%error, "app token fallback could not read due apps");
                return false;
            }
        };
        let mut stored = false;
        let mut core_down = false;
        for app in due {
            if !core_down {
                match self.core.app_token(app.id).await {
                    Ok(reply) => {
                        self.core_answered();
                        if let Some(token) = reply.token
                            && let Some(expires_at) = outlives_due_window(&token)
                        {
                            stored |= self
                                .store(
                                    app.id,
                                    token.generation,
                                    &token.access_token,
                                    expires_at,
                                    "",
                                )
                                .await;
                        }
                        continue;
                    }
                    Err(_) if self.core_down_long_enough() => core_down = true,
                    Err(_) => return stored,
                }
            }
            stored |= self.issue(app.id).await;
        }
        stored
    }

    fn probe_due(&self) -> bool {
        let mut clock = self.clock.lock().unwrap_or_else(|error| error.into_inner());
        if clock
            .probed_at
            .is_some_and(|probed_at| probed_at.elapsed() < PROBE_EVERY)
        {
            return false;
        }
        clock.probed_at = Some(Instant::now());
        true
    }

    fn core_answered(&self) {
        let mut clock = self.clock.lock().unwrap_or_else(|error| error.into_inner());
        clock.core_down_since = None;
    }

    fn core_down_long_enough(&self) -> bool {
        let mut clock = self.clock.lock().unwrap_or_else(|error| error.into_inner());
        clock
            .core_down_since
            .get_or_insert_with(Instant::now)
            .elapsed()
            >= CORE_DOWN_FOR
    }

    async fn issue(&self, oauth_app_id: Uuid) -> bool {
        if !self.claim(oauth_app_id).await {
            return false;
        }
        let app = match self.due(Some(oauth_app_id)).await {
            Ok(mut due) => match due.pop() {
                Some(app) => app,
                None => return false,
            },
            Err(error) => {
                warn!(%error, %oauth_app_id, "app token fallback could not recheck the app");
                return false;
            }
        };
        let credentials = OAuthCredentials {
            client_id: app.client_id,
            client_secret: app.client_secret,
            redirect_uri: app.redirect_uri,
        };
        let token = match self.sc.client_credentials_token(&credentials).await {
            Ok(token) if !token.access_token.is_empty() && token.expires_in > 0 => token,
            Ok(_) => {
                warn!(%oauth_app_id, "app token fallback got an incomplete token");
                return false;
            }
            Err(error) => {
                warn!(%oauth_app_id, %error, "app token fallback issuance failed");
                return false;
            }
        };
        let expires_at = Utc::now() + chrono::Duration::seconds(token.expires_in);
        let stored = self
            .store(
                app.id,
                Uuid::new_v4(),
                &token.access_token,
                expires_at,
                &token.scope,
            )
            .await;
        if stored {
            info!(%oauth_app_id, %expires_at, "app token issued on this node while core is down");
        }
        stored
    }

    async fn claim(&self, oauth_app_id: Uuid) -> bool {
        let Ok(mut connection) = self.redis.get().await else {
            return false;
        };
        let claimed: Result<Option<String>, _> = deadpool_redis::redis::cmd("SET")
            .arg(format!("auth:app_token_fallback:{oauth_app_id}"))
            .arg(1)
            .arg("NX")
            .arg("EX")
            .arg(ISSUE_LOCK_SECONDS)
            .query_async(&mut connection)
            .await;
        matches!(claimed, Ok(Some(_)))
    }

    async fn due(&self, oauth_app_id: Option<Uuid>) -> Result<Vec<DueApp>, sqlx::Error> {
        sqlx::query_file_as!(
            DueApp,
            "queries/oauth_apps/token_service/fallback_due.sql",
            Utc::now() + DUE_WITHIN,
            oauth_app_id
        )
        .fetch_all(&self.pg)
        .await
    }

    async fn store(
        &self,
        oauth_app_id: Uuid,
        generation: Uuid,
        access_token: &str,
        expires_at: DateTime<Utc>,
        scope: &str,
    ) -> bool {
        match sqlx::query_file_scalar!(
            "queries/oauth_apps/token_service/store_local.sql",
            oauth_app_id,
            generation,
            access_token,
            expires_at,
            scope
        )
        .fetch_optional(&self.pg)
        .await
        {
            Ok(stored) => stored.is_some(),
            Err(error) => {
                warn!(%oauth_app_id, %error, "app token fallback could not store the token");
                false
            }
        }
    }
}

fn outlives_due_window(token: &AppToken) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(token.expires_at_unix_ms)
        .filter(|expires_at| *expires_at > Utc::now() + DUE_WITHIN)
}

#[cfg(test)]
#[path = "fallback_tests.rs"]
mod tests;
