use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub(super) struct DuePlaylist {
    pub(super) playlist_urn: String,
    pub(super) due_at: DateTime<Utc>,
}

pub(super) async fn claim_star_owned(
    pool: &PgPool,
    owner_share: i64,
    limit: i64,
    claim_seconds: i64,
) -> Result<Vec<DuePlaylist>, sqlx::Error> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(
        DuePlaylist,
        "queries/playlist_observe/claim_due_star_playlists.sql",
        owner_share.max(1),
        limit,
        claim_seconds as f64
    )
    .fetch_all(pool)
    .await
}

pub(super) async fn waiting(pool: &PgPool, limit: i64, priority: i16) -> Result<i64, sqlx::Error> {
    sqlx::query_file_scalar!(
        "queries/playlist_observe/background_backlog.sql",
        limit,
        priority
    )
    .fetch_one(pool)
    .await
}
