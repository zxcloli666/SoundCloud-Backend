use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub(super) struct NextWaiting {
    pub(super) star: Option<DateTime<Utc>>,
    pub(super) regular: Option<DateTime<Utc>>,
}

pub(super) async fn claim_star_owned(
    pool: &PgPool,
    owner_share: i64,
    limit: i64,
    claim_seconds: i64,
) -> Result<Vec<String>, sqlx::Error> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    sqlx::query_file_scalar!(
        "queries/playlist_observe/claim_due_star_playlists.sql",
        owner_share.max(1),
        limit,
        claim_seconds as f64
    )
    .fetch_all(pool)
    .await
}

pub(super) async fn next_waiting(pool: &PgPool) -> Result<NextWaiting, sqlx::Error> {
    let next = sqlx::query_file!("queries/playlist_observe/due_next.sql")
        .fetch_one(pool)
        .await?;
    Ok(NextWaiting {
        star: next.star,
        regular: next.regular,
    })
}
