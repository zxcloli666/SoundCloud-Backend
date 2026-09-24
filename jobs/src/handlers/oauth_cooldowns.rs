use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct OAuthAppCooldowns {
    pool: PgPool,
}

impl OAuthAppCooldowns {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn retry_after_seconds(
        &self,
        oauth_app_id: Uuid,
    ) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_file_scalar!("queries/oauth_cooldowns/retry_after.sql", oauth_app_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn penalize(
        &self,
        oauth_app_id: Uuid,
        minimum_seconds: i64,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_file_scalar!(
            "queries/oauth_cooldowns/penalize.sql",
            oauth_app_id,
            minimum_seconds
        )
        .fetch_one(&self.pool)
        .await
    }
}
