mod queued;
mod round;
#[cfg(test)]
mod tests;
mod tier;
mod turns;

use sqlx::{PgConnection, PgPool};

use super::indexing::AUDIO_INDEX_QUARANTINE_SECONDS;
use crate::queue::{JobError, JobResult};

pub(crate) use self::queued::record_queued_waits;
pub(crate) use self::round::{Round, split_next, with_next};
pub(crate) use self::tier::{
    CORE_FAST_TRACK_PRIORITY, LIKED_PRIORITY, LYRICS_EMBED_PRIORITY, Pass, Tier, of_track,
    of_tracks,
};
pub(crate) use self::turns::{Turn, Turns};

const HISTORY_DAYS: i32 = 30;
const QUALITY_UPGRADE_COOLDOWN_SECONDS: i64 = 6 * 60 * 60;
pub(crate) const STAR_SCAN_WINDOW: i64 = 2_000;

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
        let due = sqlx::query_file!(
            "queries/star_priority/refresh_due.sql",
            AUDIO_INDEX_QUARANTINE_SECONDS,
            QUALITY_UPGRADE_COOLDOWN_SECONDS
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
            due_changed = due.changed,
            audio_due = due.audio_due,
            lookup_due = due.lookup_due,
            embed_due = due.embed_due,
            align_due = due.align_due,
            enrich_due = due.enrich_due,
            upgrade_due = due.upgrade_due,
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
