use std::sync::Arc;

use backend_contracts::{JobKind, PlaylistObservePayload};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::background_jobs::{BackgroundJob, BackgroundJobs};
use crate::error::AppResult;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistMembershipStatus {
    pub baseline_generation: i64,
    pub projection_revision: i64,
    pub projection_track_count: i32,
    pub last_operation_sequence: i64,
    pub committed_operation_sequence: i64,
    pub pending_operations: i64,
    pub conflicted_operations: i64,
    pub status: String,
    pub conflict_code: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
}

fn membership_status(row: Option<MembershipStatusRow>) -> PlaylistMembershipStatus {
    let Some(row) = row else {
        return PlaylistMembershipStatus {
            baseline_generation: 0,
            projection_revision: 0,
            projection_track_count: 0,
            last_operation_sequence: 0,
            committed_operation_sequence: 0,
            pending_operations: 0,
            conflicted_operations: 0,
            status: "unhydrated".to_owned(),
            conflict_code: None,
            observed_at: None,
        };
    };
    PlaylistMembershipStatus {
        baseline_generation: row.baseline_generation,
        projection_revision: row.projection_revision,
        projection_track_count: row.projection_track_count,
        last_operation_sequence: row.last_operation_sequence,
        committed_operation_sequence: row.committed_operation_sequence,
        pending_operations: row.pending_operations,
        conflicted_operations: row.conflicted_operations,
        status: row.sync_status,
        conflict_code: row.conflict_code,
        observed_at: row.observed_at,
    }
}

struct MembershipStatusRow {
    baseline_generation: i64,
    projection_revision: i64,
    projection_track_count: i32,
    last_operation_sequence: i64,
    committed_operation_sequence: i64,
    sync_status: String,
    conflict_code: Option<String>,
    observed_at: Option<DateTime<Utc>>,
    pending_operations: i64,
    conflicted_operations: i64,
}

pub struct PlaylistMembership {
    pool: PgPool,
    jobs: Arc<BackgroundJobs>,
}

impl PlaylistMembership {
    pub fn new(pool: PgPool, jobs: Arc<BackgroundJobs>) -> Self {
        Self { pool, jobs }
    }

    pub async fn status(&self, playlist_urn: &str) -> AppResult<PlaylistMembershipStatus> {
        let row = load_membership_status(&self.pool, playlist_urn).await?;
        if row.is_none() {
            tracing::warn!(
                playlist_urn,
                "playlist has no membership state; answering as unhydrated"
            );
        }
        Ok(membership_status(row))
    }

    pub async fn enqueue_observation_if_due(&self, playlist_urn: &str) {
        let claim = claim_observation_enqueue(&self.pool, playlist_urn).await;
        let Ok(Some(claimed_until)) = claim else {
            return;
        };

        let job = BackgroundJob::coalescing(
            JobKind::PlaylistObserveShadow,
            playlist_urn,
            PlaylistObservePayload {
                playlist_urn: playlist_urn.to_owned(),
            },
        )
        .map(BackgroundJob::if_absent);
        let published = match job {
            Ok(job) => self.jobs.enqueue_opportunistic(&job).await,
            Err(_) => false,
        };
        if published {
            return;
        }

        let _ = release_observation_enqueue(&self.pool, playlist_urn, claimed_until).await;
    }
}

async fn load_membership_status(
    pool: &PgPool,
    playlist_urn: &str,
) -> Result<Option<MembershipStatusRow>, sqlx::Error> {
    sqlx::query_file_as!(
        MembershipStatusRow,
        "queries/playlists/membership_status.sql",
        playlist_urn
    )
    .fetch_optional(pool)
    .await
}

async fn claim_observation_enqueue(
    pool: &PgPool,
    playlist_urn: &str,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_file_scalar!("queries/playlists/claim_observe_enqueue.sql", playlist_urn)
        .fetch_optional(pool)
        .await
}

async fn release_observation_enqueue(
    pool: &PgPool,
    playlist_urn: &str,
    claimed_until: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!(
        "queries/playlists/release_observe_enqueue.sql",
        playlist_urn,
        claimed_until
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::playlists::test_schema;

    const PLAYLIST: &str = "soundcloud:playlists:42";
    const OWNER: &str = "17";

    #[sqlx::test(migrations = false)]
    async fn observation_enqueue_is_throttled_and_count_mismatch_breaks_the_throttle(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        test_schema::install(&pool).await?;
        test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
        sqlx::query("UPDATE playlist_membership_state SET next_reconcile_at = now()")
            .execute(&pool)
            .await?;

        assert!(claim_observation_enqueue(&pool, PLAYLIST).await?.is_some());
        assert!(claim_observation_enqueue(&pool, PLAYLIST).await?.is_none());

        sqlx::query("UPDATE playlists SET track_count = 3")
            .execute(&pool)
            .await?;
        sqlx::query_file!(
            "../utils/catalog-ingest/queries/playlists/ensure_membership_state.sql",
            PLAYLIST
        )
        .execute(&pool)
        .await?;
        assert!(claim_observation_enqueue(&pool, PLAYLIST).await?.is_some());
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn status_reports_projection_and_pending_operations(pool: PgPool) -> anyhow::Result<()> {
        test_schema::install(&pool).await?;
        let observation_id =
            test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
        sqlx::query(
            "UPDATE playlist_membership_state
             SET projection_revision = 3,
                 last_operation_sequence = 2,
                 committed_operation_sequence = 1,
                 sync_status = 'conflict',
                 conflict_code = 'legacy_order_only'
             WHERE playlist_urn = $1",
        )
        .bind(PLAYLIST)
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO playlist_membership_operations (
                 operation_id, playlist_urn, sequence, actor_sc_user_id, idempotency_key,
                 request_fingerprint, base_baseline_generation, base_observation_id,
                 expected_projection_revision, accepted_projection_revision,
                 kind, track_id, boundary
             ) VALUES ($1, $2, 2, $3, $4, sha256('x'::bytea), 1, $5, 2, 3, 'add', '9', 'back')",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(PLAYLIST)
        .bind(OWNER)
        .bind(uuid::Uuid::now_v7())
        .bind(observation_id)
        .execute(&pool)
        .await?;

        let row = load_membership_status(&pool, PLAYLIST)
            .await?
            .expect("membership state");
        assert_eq!(row.baseline_generation, 1);
        assert_eq!(row.projection_revision, 3);
        assert_eq!(row.projection_track_count, 2);
        assert_eq!(row.last_operation_sequence, 2);
        assert_eq!(row.committed_operation_sequence, 1);
        assert_eq!(row.sync_status, "conflict");
        assert_eq!(row.pending_operations, 1);
        assert_eq!(row.conflicted_operations, 0);
        assert!(row.observed_at.is_some());
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_playlist_that_was_never_observed_still_reports_its_status(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id)
             VALUES ('77', $1, 'Fresh', 'fresh', $2)",
        )
        .bind(PLAYLIST)
        .bind(OWNER)
        .execute(&pool)
        .await?;
        sqlx::query("INSERT INTO playlist_membership_state (playlist_urn) VALUES ($1)")
            .bind(PLAYLIST)
            .execute(&pool)
            .await?;

        let row = load_membership_status(&pool, PLAYLIST)
            .await?
            .expect("membership state");
        assert!(
            row.observed_at.is_none(),
            "a playlist with no observation must report an absent timestamp, not fail to decode"
        );
        assert_eq!(row.sync_status, "unhydrated");
        assert_eq!(row.pending_operations, 0);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_playlist_whose_state_row_is_absent_is_still_served(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id)
             VALUES ('78', $1, 'Stateless', 'stateless', $2)",
        )
        .bind(PLAYLIST)
        .bind(OWNER)
        .execute(&pool)
        .await?;

        let row = load_membership_status(&pool, PLAYLIST).await?;
        assert!(row.is_none(), "this playlist deliberately has no state row");

        let status = membership_status(row);

        assert_eq!(
            status.status, "unhydrated",
            "a missing row is metadata about syncing, not a reason to refuse the tracks the \
             listener asked for; this used to answer 500 for the whole route"
        );
        assert_eq!(status.projection_track_count, 0);
        assert_eq!(status.pending_operations, 0);
        Ok(())
    }
}
