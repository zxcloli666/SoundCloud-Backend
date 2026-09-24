mod snapshot;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use sqlx::{FromRow, PgPool};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

use self::snapshot::TokenSnapshot;

const MIN_FRESH: chrono::Duration = chrono::Duration::seconds(30);
const SNAPSHOT_MAX_AGE: Duration = Duration::from_secs(15);
const RELOAD_BACKOFF_MIN: Duration = Duration::from_secs(1);
const RELOAD_BACKOFF_MAX: Duration = Duration::from_secs(15);

#[derive(Clone, FromRow)]
pub(crate) struct PublicToken {
    pub oauth_app_id: Uuid,
    pub generation: Uuid,
    pub access_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PublicTokenId {
    pub oauth_app_id: Uuid,
    pub generation: Uuid,
}

impl PublicToken {
    pub fn id(&self) -> PublicTokenId {
        PublicTokenId {
            oauth_app_id: self.oauth_app_id,
            generation: self.generation,
        }
    }
}

pub struct OAuthAppTokenService {
    pg: PgPool,
    snapshot: TokenSnapshot,
    reload_lock: Mutex<()>,
}

impl OAuthAppTokenService {
    pub fn new(pg: PgPool) -> Arc<Self> {
        Arc::new(Self {
            pg,
            snapshot: TokenSnapshot::default(),
            reload_lock: Mutex::new(()),
        })
    }

    pub(crate) async fn snapshot(&self) -> AppResult<Vec<PublicToken>> {
        self.snapshot_with(|| self.load()).await
    }

    pub(crate) async fn reject(&self, token: PublicTokenId) {
        self.snapshot.reject(token);
        match self.persist_rejection(token).await {
            Ok(true) => {}
            Ok(false) => self.snapshot.resolve_rejection(token),
            Err(error) => {
                warn!(
                    oauth_app_id = %token.oauth_app_id,
                    generation = %token.generation,
                    error = %error,
                    "failed to persist rejected OAuth app token"
                );
            }
        }
    }

    async fn persist_rejection(&self, token: PublicTokenId) -> Result<bool, sqlx::Error> {
        let updated = sqlx::query_file_scalar!(
            "queries/oauth_apps/token_service/reject_generation.sql",
            token.oauth_app_id,
            token.generation
        )
        .fetch_optional(&self.pg)
        .await?;
        Ok(updated.is_some())
    }

    async fn snapshot_with<F, Fut, E>(&self, load: F) -> AppResult<Vec<PublicToken>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<PublicToken>, E>>,
        E: std::fmt::Display,
    {
        if let Err(error) = self.reload_if_due_with(load).await {
            warn!(error = %error, "failed to reload OAuth app token snapshot");
        }
        let tokens = self.snapshot.tokens_fresh_after(Utc::now() + MIN_FRESH);
        if tokens.is_empty() {
            return Err(AppError::soundcloud_temporarily_unavailable());
        }
        Ok(shuffled(tokens))
    }

    async fn load(&self) -> Result<Vec<PublicToken>, sqlx::Error> {
        for rejected in self.snapshot.rejected() {
            if !self.persist_rejection(rejected).await? {
                self.snapshot.resolve_rejection(rejected);
            }
        }
        let cutoff = Utc::now() + MIN_FRESH;
        sqlx::query_file_as!(
            PublicToken,
            "queries/oauth_apps/token_service/reload_snapshot.sql",
            cutoff
        )
        .fetch_all(&self.pg)
        .await
    }

    async fn reload_if_due_with<F, Fut, E>(&self, load: F) -> Result<(), E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<PublicToken>, E>>,
    {
        if !self.snapshot.reload_due(SNAPSHOT_MAX_AGE) {
            return Ok(());
        }
        let _guard = self.reload_lock.lock().await;
        if !self.snapshot.reload_due(SNAPSHOT_MAX_AGE) {
            return Ok(());
        }
        match load().await {
            Ok(tokens) => {
                self.snapshot.replace(tokens);
                Ok(())
            }
            Err(error) => {
                self.snapshot
                    .record_reload_failure(RELOAD_BACKOFF_MIN, RELOAD_BACKOFF_MAX);
                Err(error)
            }
        }
    }
}

fn shuffled(mut tokens: Vec<PublicToken>) -> Vec<PublicToken> {
    tokens.shuffle(&mut rand::thread_rng());
    tokens
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sqlx::postgres::PgPoolOptions;
    use tokio::task::JoinSet;

    use super::*;

    fn token(expires_at: DateTime<Utc>) -> PublicToken {
        PublicToken {
            oauth_app_id: Uuid::from_u128(1),
            generation: Uuid::from_u128(2),
            access_token: "public-token".to_owned(),
            expires_at,
        }
    }

    fn service() -> Arc<OAuthAppTokenService> {
        OAuthAppTokenService::new(
            PgPoolOptions::new()
                .connect_lazy("postgres://localhost/unused")
                .expect("test database URL should parse"),
        )
    }

    #[tokio::test]
    async fn stale_cache_survives_a_reload_failure_while_token_is_valid() {
        let service = service();
        service.snapshot.replace_stale(
            vec![token(Utc::now() + chrono::Duration::hours(1))],
            SNAPSHOT_MAX_AGE + Duration::from_secs(1),
        );

        let tokens = service
            .snapshot_with(|| async { Err::<Vec<PublicToken>, _>("database unavailable") })
            .await
            .expect("valid cached token should remain available");

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].access_token, "public-token");
    }

    #[tokio::test]
    async fn expired_cache_fails_closed_after_a_reload_failure() {
        let service = service();
        service.snapshot.replace_stale(
            vec![token(Utc::now() + chrono::Duration::seconds(20))],
            SNAPSHOT_MAX_AGE + Duration::from_secs(1),
        );

        let result = service
            .snapshot_with(|| async { Err::<Vec<PublicToken>, _>("database unavailable") })
            .await;

        assert!(matches!(
            result,
            Err(AppError::SoundCloudTemporarilyUnavailable { .. })
        ));
    }

    #[tokio::test]
    async fn concurrent_stale_reads_share_one_reload_attempt() {
        let service = service();
        let attempts = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();

        for _ in 0..32 {
            let service = Arc::clone(&service);
            let attempts = Arc::clone(&attempts);
            tasks.spawn(async move {
                service
                    .snapshot_with(|| async move {
                        attempts.fetch_add(1, Ordering::Relaxed);
                        Err::<Vec<PublicToken>, _>("database unavailable")
                    })
                    .await
            });
        }
        while let Some(result) = tasks.join_next().await {
            assert!(result.expect("snapshot task should finish").is_err());
        }

        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[sqlx::test(migrations = false)]
    async fn rejection_marks_only_the_observed_generation_due(pool: PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE oauth_app_tokens (
                 oauth_app_id uuid PRIMARY KEY,
                 generation uuid NOT NULL,
                 expires_at timestamptz NOT NULL
             )",
        )
        .execute(&pool)
        .await?;
        let app_id = Uuid::from_u128(1);
        let observed = PublicTokenId {
            oauth_app_id: app_id,
            generation: Uuid::from_u128(2),
        };
        sqlx::query(
            "INSERT INTO oauth_app_tokens (oauth_app_id, generation, expires_at)
             VALUES ($1, $2, now() + interval '1 hour')",
        )
        .bind(app_id)
        .bind(observed.generation)
        .execute(&pool)
        .await?;
        let service = OAuthAppTokenService::new(pool.clone());

        service.reject(observed).await;

        let rejected_is_due: bool = sqlx::query_scalar(
            "SELECT expires_at <= now() FROM oauth_app_tokens WHERE oauth_app_id = $1",
        )
        .bind(app_id)
        .fetch_one(&pool)
        .await?;
        assert!(rejected_is_due);

        let replacement = Uuid::from_u128(3);
        sqlx::query(
            "UPDATE oauth_app_tokens
             SET generation = $2, expires_at = now() + interval '1 hour'
             WHERE oauth_app_id = $1",
        )
        .bind(app_id)
        .bind(replacement)
        .execute(&pool)
        .await?;

        service.reject(observed).await;

        let replacement_still_valid: bool = sqlx::query_scalar(
            "SELECT generation = $2 AND expires_at > now() + interval '30 minutes'
             FROM oauth_app_tokens
             WHERE oauth_app_id = $1",
        )
        .bind(app_id)
        .bind(replacement)
        .fetch_one(&pool)
        .await?;
        assert!(replacement_still_valid);
        Ok(())
    }

    #[test]
    fn rejected_generation_stays_filtered_until_it_is_replaced() {
        let snapshot = TokenSnapshot::default();
        let rejected = token(Utc::now() + chrono::Duration::hours(1));
        snapshot.replace(vec![rejected.clone()]);
        snapshot.reject(rejected.id());
        snapshot.replace(vec![rejected.clone()]);

        assert!(snapshot.tokens_fresh_after(Utc::now()).is_empty());

        let replacement = PublicToken {
            generation: Uuid::from_u128(3),
            ..rejected
        };
        snapshot.replace(vec![replacement.clone()]);

        assert_eq!(snapshot.tokens_fresh_after(Utc::now()).len(), 1);
        assert_eq!(
            snapshot.tokens_fresh_after(Utc::now())[0].id(),
            replacement.id()
        );
    }
}
