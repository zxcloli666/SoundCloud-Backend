mod queued;
mod round;
#[cfg(test)]
mod tests;
mod tier;
mod turns;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::{PgConnection, PgPool};

use super::indexing::AUDIO_INDEX_QUARANTINE_SECONDS;
use crate::metrics::{StarDuePath, record_star_due};
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
const DUE_SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
pub(crate) const STAR_SCAN_WINDOW: i64 = 2_000;

pub(crate) struct StarTiersHandler {
    pool: PgPool,
    enrich_max_attempts: i16,
    due_swept_at: Mutex<Option<Instant>>,
}

impl StarTiersHandler {
    pub(crate) fn new(pool: PgPool, enrich_max_attempts: i16) -> Self {
        Self {
            pool,
            enrich_max_attempts,
            due_swept_at: Mutex::new(None),
        }
    }

    pub(crate) async fn refresh(&self) -> JobResult {
        let refreshed = sqlx::query_file!(
            "queries/star_priority/refresh_track_tiers.sql",
            HISTORY_DAYS
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let started = Instant::now();
        let sweep = self.due_swept_at.lock().map_or(true, |swept_at| {
            swept_at.is_none_or(|at| at.elapsed() >= DUE_SWEEP_EVERY)
        });
        let due = sqlx::query_file!(
            "queries/star_priority/refresh_due.sql",
            AUDIO_INDEX_QUARANTINE_SECONDS,
            QUALITY_UPGRADE_COOLDOWN_SECONDS,
            self.enrich_max_attempts,
            sweep
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if sweep && let Ok(mut swept_at) = self.due_swept_at.lock() {
            *swept_at = Some(started);
        }
        let path = if sweep {
            StarDuePath::Sweep
        } else {
            StarDuePath::Dirty
        };
        record_star_due(path, due.rechecked, due.changed, due.missed);
        tracing::info!(
            star_users = refreshed.star_users,
            star_liked = refreshed.star_liked,
            star_other = refreshed.star_other,
            written = refreshed.written,
            removed = refreshed.removed,
            due_path = path.as_str(),
            due_rechecked = due.rechecked,
            due_written = due.written,
            due_changed = due.changed,
            due_missed = due.missed,
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
