mod picks;
#[cfg(test)]
mod tests;
mod tier;
mod turns;

use sqlx::{PgConnection, PgPool};

use crate::queue::{JobError, JobResult};

pub(crate) use self::picks::Picks;
pub(crate) use self::tier::{CORE_FAST_TRACK_PRIORITY, Pass, Tier, of_track, of_tracks};
pub(crate) use self::turns::{Turn, Turns};

const HISTORY_DAYS: i32 = 30;

pub(crate) struct StarTiersHandler {
    pool: PgPool,
}

impl StarTiersHandler {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) async fn refresh(&self) -> JobResult {
        let refreshed = sqlx::query_file!(
            "queries/star_priority/refresh_track_tiers.sql",
            HISTORY_DAYS
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        tracing::info!(
            star_users = refreshed.star_users,
            star_liked = refreshed.star_liked,
            star_other = refreshed.star_other,
            written = refreshed.written,
            removed = refreshed.removed,
            "star track tiers refreshed"
        );
        Ok(())
    }
}

pub(crate) async fn mark_liked_tracks(
    connection: &mut PgConnection,
    user_id: &str,
    sc_track_ids: &[String],
) -> Result<(), sqlx::Error> {
    if sc_track_ids.is_empty() {
        return Ok(());
    }
    sqlx::query_file!(
        "queries/star_priority/mark_liked_tracks.sql",
        user_id,
        sc_track_ids
    )
    .execute(connection)
    .await?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn install_star_track_tiers(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0151_star_track_tiers.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}
