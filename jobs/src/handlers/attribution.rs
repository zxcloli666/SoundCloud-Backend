#[cfg(test)]
#[path = "attribution_tests.rs"]
mod tests;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::queue::{JobError, JobResult};

const WALKER_BATCH: i64 = 1_000;
const IDENTITY_BATCH: i64 = 500;

pub struct AttributionHandler {
    pool: PgPool,
}

struct RevalidationState {
    walker_completed_at: Option<DateTime<Utc>>,
    identity_cursor_artist: Option<Uuid>,
    identity_cursor_account: Option<String>,
    identity_completed_at: Option<DateTime<Utc>>,
}

struct UnverifiedLink {
    artist_id: Uuid,
    sc_user_id: String,
}

struct Removed {
    credits_removed: i64,
    tracks_reset: i64,
}

impl AttributionHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn revalidate(&self) -> JobResult {
        let state = self.load_state().await?;
        if state.walker_completed_at.is_none() {
            return self.revalidate_walker_credits().await;
        }
        if state.identity_completed_at.is_none() {
            return self.revalidate_identity_claims(&state).await;
        }
        Ok(())
    }

    async fn load_state(&self) -> Result<RevalidationState, JobError> {
        sqlx::query_file_as!(RevalidationState, "queries/attribution/load_state.sql")
            .fetch_optional(&self.pool)
            .await
            .map_err(JobError::retryable)?
            .ok_or_else(|| {
                JobError::permanent(anyhow::anyhow!(
                    "artist attribution revalidation state is missing"
                ))
            })
    }

    async fn revalidate_walker_credits(&self) -> JobResult {
        let removed = sqlx::query_file_as!(
            Removed,
            "queries/attribution/revalidate_walker_credits.sql",
            WALKER_BATCH
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        if removed.credits_removed == 0 {
            sqlx::query_file!("queries/attribution/complete_walker_phase.sql")
                .execute(&self.pool)
                .await
                .map_err(JobError::retryable)?;
            info!("walker attribution revalidation finished");
            return Ok(());
        }

        sqlx::query_file!(
            "queries/attribution/record_progress.sql",
            removed.credits_removed,
            removed.tracks_reset
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        info!(
            credits = removed.credits_removed,
            tracks = removed.tracks_reset,
            "walker credits removed from artist catalogs"
        );
        Ok(())
    }

    async fn revalidate_identity_claims(&self, state: &RevalidationState) -> JobResult {
        let link = sqlx::query_file_as!(
            UnverifiedLink,
            "queries/attribution/next_unverified_link.sql",
            state.identity_cursor_artist,
            state.identity_cursor_account
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let Some(link) = link else {
            sqlx::query_file!("queries/attribution/complete_identity_phase.sql")
                .execute(&self.pool)
                .await
                .map_err(JobError::retryable)?;
            info!("identity attribution revalidation finished");
            return Ok(());
        };

        let removed = sqlx::query_file_as!(
            Removed,
            "queries/attribution/reset_identity_tracks.sql",
            link.artist_id,
            link.sc_user_id,
            IDENTITY_BATCH
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        if removed.tracks_reset >= IDENTITY_BATCH {
            sqlx::query_file!(
                "queries/attribution/record_progress.sql",
                removed.credits_removed,
                removed.tracks_reset
            )
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
            return Ok(());
        }

        sqlx::query_file!(
            "queries/attribution/advance_identity_cursor.sql",
            link.artist_id,
            link.sc_user_id,
            removed.credits_removed,
            removed.tracks_reset
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        if removed.tracks_reset > 0 {
            info!(
                artist = %link.artist_id,
                account = %link.sc_user_id,
                tracks = removed.tracks_reset,
                "unverified account claims returned to enrichment"
            );
        }
        Ok(())
    }
}
