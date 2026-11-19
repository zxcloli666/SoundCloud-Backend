mod queued;
mod round;
#[cfg(test)]
mod tests;
mod tier;
mod turns;

use std::sync::Mutex;

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
const SWEEP_ROWS: i64 = 100;
const CATCH_UP_ROWS: i64 = 1_000;
pub(crate) const STAR_SCAN_WINDOW: i64 = 2_000;

pub(crate) struct StarTiersHandler {
    pool: PgPool,
    enrich_max_attempts: i16,
    sweep: Mutex<Sweep>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Sweep {
    after: String,
    catching_up: bool,
}

impl Sweep {
    fn rows(&self) -> i64 {
        if self.catching_up {
            CATCH_UP_ROWS
        } else {
            SWEEP_ROWS
        }
    }

    fn next(self, lapsed: bool, swept_through: Option<String>) -> Self {
        if lapsed {
            return Self {
                after: String::new(),
                catching_up: true,
            };
        }
        match swept_through {
            Some(after) => Self { after, ..self },
            None => Self::default(),
        }
    }
}

impl StarTiersHandler {
    pub(crate) fn new(pool: PgPool, enrich_max_attempts: i16) -> Self {
        Self {
            pool,
            enrich_max_attempts,
            sweep: Mutex::new(Sweep::default()),
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
        let sweep = self
            .sweep
            .lock()
            .map(|sweep| sweep.clone())
            .unwrap_or_default();
        let due = sqlx::query_file!(
            "queries/star_priority/refresh_due.sql",
            AUDIO_INDEX_QUARANTINE_SECONDS,
            QUALITY_UPGRADE_COOLDOWN_SECONDS,
            self.enrich_max_attempts,
            sweep.after,
            sweep.rows()
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let next = sweep.clone().next(due.lapsed, due.swept_through);
        let catching_up = next.catching_up;
        if let Ok(mut current) = self.sweep.lock() {
            *current = next;
        }
        record_star_due(StarDuePath::Dirty, due.rechecked, due.changed, 0);
        record_star_due(StarDuePath::Sweep, due.swept, due.missed, due.missed);
        tracing::info!(
            star_users = refreshed.star_users,
            star_liked = refreshed.star_liked,
            star_other = refreshed.star_other,
            written = refreshed.written,
            removed = refreshed.removed,
            due_rechecked = due.rechecked,
            due_written = due.written,
            due_changed = due.changed,
            due_swept = due.swept,
            due_missed = due.missed,
            due_lapsed = due.lapsed,
            sweep_after = sweep.after.as_str(),
            sweep_catching_up = catching_up,
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
