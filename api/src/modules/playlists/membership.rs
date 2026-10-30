use backend_contracts::{JobKind, PlaylistObservePayload, STAR_PRIORITY_BONUS};
use chrono::{DateTime, Utc};
use deadpool_redis::Pool;
use serde::Serialize;
use sqlx::PgPool;
use wt::ObserveClaim;

use crate::background_jobs::queue;
use crate::error::AppResult;
use crate::modules::cold_refresh::VIEWER_PRIORITY;

const EDIT_PRIORITY: i16 = 30;
const FAILURE_STREAK_LIMIT: i32 = 5;
const VIEW_CLAIM_SECS: u64 = 30;

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
    pub queued_since: Option<DateTime<Utc>>,
    pub error_code: Option<&'static str>,
    pub retry_after_seconds: i64,
}

fn playlist_error_code(
    status: &str,
    conflict_code: Option<&str>,
    streak: i32,
    last_error: Option<&str>,
) -> Option<&'static str> {
    match (status, conflict_code) {
        ("auth_required", _) => Some("auth_required"),
        (_, Some("remote_not_found")) => Some("not_found"),
        ("retry_wait", _) if streak >= FAILURE_STREAK_LIMIT => match last_error {
            Some("soundcloud_forbidden") => Some("forbidden"),
            _ => Some("unavailable"),
        },
        _ => None,
    }
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
            queued_since: None,
            error_code: None,
            retry_after_seconds: 0,
        };
    };
    let error_code = playlist_error_code(
        &row.sync_status,
        row.conflict_code.as_deref(),
        row.reconcile_failure_streak,
        row.last_error.as_deref(),
    );
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
        queued_since: row.queued_since,
        error_code,
        retry_after_seconds: row.retry_after_seconds,
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
    reconcile_failure_streak: i32,
    last_error: Option<String>,
    retry_after_seconds: i64,
    queued_since: Option<DateTime<Utc>>,
}

pub struct PlaylistMembership {
    pool: PgPool,
    redis: Pool,
}

impl PlaylistMembership {
    pub fn new(pool: PgPool, redis: Pool) -> Self {
        Self { pool, redis }
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

    pub async fn track_if_untracked(&self, playlist_urn: &str) -> AppResult<()> {
        sqlx::query_file!(
            "queries/playlists/ensure_membership_tracked.sql",
            playlist_urn
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn enqueue_observation_if_due(&self, playlist_urn: &str) {
        let Ok(Some(due_at)) = sqlx::query_file_scalar!(
            "queries/playlists/observe_due_at.sql",
            playlist_urn,
            wt::now()
        )
        .fetch_optional(&self.pool)
        .await
        else {
            return;
        };
        if !self.claim_view(playlist_urn, due_at).await {
            return;
        }
        self.enqueue(playlist_urn, VIEWER_PRIORITY, true).await;
    }

    pub async fn enqueue_edit_observation(&self, playlist_urn: &str) -> bool {
        self.enqueue(playlist_urn, EDIT_PRIORITY, false).await
    }

    async fn claim_view(&self, playlist_urn: &str, due_at: DateTime<Utc>) -> bool {
        let Ok(mut connection) = self.redis.get().await else {
            return true;
        };
        let claimed: Result<Option<String>, _> = deadpool_redis::redis::cmd("SET")
            .arg(format!(
                "playlist:observe:{playlist_urn}:{}",
                due_at.timestamp_micros()
            ))
            .arg(1)
            .arg("NX")
            .arg("EX")
            .arg(VIEW_CLAIM_SECS)
            .query_async(&mut connection)
            .await;
        !matches!(claimed, Ok(None))
    }

    async fn enqueue(&self, playlist_urn: &str, priority: i16, claim: bool) -> bool {
        let priority = owner_priority(&self.pool, playlist_urn, priority)
            .await
            .unwrap_or(priority);
        let Ok(job) = queue::job(
            JobKind::PlaylistObserveShadow,
            playlist_urn.to_owned(),
            PlaylistObservePayload {
                playlist_urn: playlist_urn.to_owned(),
            },
            priority,
        ) else {
            return false;
        };
        let claim = claim.then(|| ObserveClaim {
            playlist_urn: playlist_urn.to_owned(),
            at: wt::now(),
        });
        queue::enqueue(&self.pool, vec![job], claim).await.is_ok()
    }
}

async fn owner_priority(
    pool: &PgPool,
    playlist_urn: &str,
    priority: i16,
) -> Result<i16, sqlx::Error> {
    sqlx::query_file_scalar!(
        "queries/playlists/observe_priority.sql",
        playlist_urn,
        priority,
        STAR_PRIORITY_BONUS
    )
    .fetch_one(pool)
    .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::playlists::test_schema;

    const PLAYLIST: &str = "soundcloud:playlists:42";
    const OWNER: &str = "17";

    async fn claimed(pool: &PgPool) -> anyhow::Result<bool> {
        let job = queue::job(
            JobKind::PlaylistObserveShadow,
            PLAYLIST.to_owned(),
            PlaylistObservePayload {
                playlist_urn: PLAYLIST.to_owned(),
            },
            0,
        )?;
        let claim = ObserveClaim {
            playlist_urn: PLAYLIST.to_owned(),
            at: wt::now(),
        };
        let enqueued = queue::enqueue(pool, vec![job], Some(claim)).await?;
        Ok(enqueued.is_some_and(|enqueued| enqueued.claimed))
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn observation_enqueue_is_throttled_and_count_mismatch_breaks_the_throttle(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        test_schema::seed_observed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
        sqlx::query("UPDATE playlist_membership_state SET next_reconcile_at = now()")
            .execute(&pool)
            .await?;

        assert!(claimed(&pool).await?);
        assert!(!claimed(&pool).await?);

        test_schema::observe(&pool, PLAYLIST, OWNER, 2).await?;
        assert!(!claimed(&pool).await?);

        test_schema::observe(&pool, PLAYLIST, OWNER, 3).await?;
        assert!(claimed(&pool).await?);

        let jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM background_jobs WHERE kind = 'playlists.observe_shadow'",
        )
        .fetch_one(&pool)
        .await?;
        let statuses: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM job_status WHERE kind = 'playlists.observe_shadow' AND state = 'queued'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!((jobs, statuses), (1, 1));
        Ok(())
    }

    async fn count_enqueue_commands(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "ALTER FUNCTION scd_cmd_job_enqueue(jsonb) RENAME TO scd_cmd_job_enqueue_real;
             CREATE TABLE enqueue_commands (n bigserial PRIMARY KEY);
             CREATE FUNCTION scd_cmd_job_enqueue(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
             BEGIN
                 INSERT INTO enqueue_commands DEFAULT VALUES;
                 RETURN scd_cmd_job_enqueue_real(args);
             END $f$;",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn enqueue_commands(pool: &PgPool) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM enqueue_commands")
            .fetch_one(pool)
            .await?)
    }

    fn node_redis(database: u8) -> Pool {
        let url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
        deadpool_redis::Config::from_url(format!("{url}/{database}"))
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_count_change_makes_the_next_view_enqueue_at_once(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        test_schema::seed_observed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
        sqlx::query(
            "UPDATE playlist_membership_state SET next_reconcile_at = now() - interval '1 hour'",
        )
        .execute(&pool)
        .await?;
        count_enqueue_commands(&pool).await?;
        let node = PlaylistMembership::new(pool.clone(), node_redis(3));
        let mut connection = node.redis.get().await?;
        let _: () = deadpool_redis::redis::cmd("FLUSHDB")
            .query_async(&mut connection)
            .await?;

        node.enqueue_observation_if_due(PLAYLIST).await;
        assert_eq!(enqueue_commands(&pool).await?, 1);

        node.enqueue_observation_if_due(PLAYLIST).await;
        assert_eq!(enqueue_commands(&pool).await?, 1);

        test_schema::observe(&pool, PLAYLIST, OWNER, 3).await?;
        node.enqueue_observation_if_due(PLAYLIST).await;
        assert_eq!(enqueue_commands(&pool).await?, 2);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_thousand_views_on_two_nodes_make_one_background_job(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        test_schema::seed_observed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
        sqlx::query(
            "UPDATE playlist_membership_state SET next_reconcile_at = now() - interval '1 hour'",
        )
        .execute(&pool)
        .await?;
        count_enqueue_commands(&pool).await?;
        let nodes = [
            std::sync::Arc::new(PlaylistMembership::new(pool.clone(), node_redis(1))),
            std::sync::Arc::new(PlaylistMembership::new(pool.clone(), node_redis(2))),
        ];
        for node in &nodes {
            let mut connection = node.redis.get().await?;
            let _: () = deadpool_redis::redis::cmd("FLUSHDB")
                .query_async(&mut connection)
                .await?;
        }

        let mut views = tokio::task::JoinSet::new();
        for view in 0..1000 {
            let node = nodes[view % 2].clone();
            views.spawn(async move { node.enqueue_observation_if_due(PLAYLIST).await });
        }
        while let Some(done) = views.join_next().await {
            done?;
        }

        let jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM background_jobs WHERE kind = 'playlists.observe_shadow' AND dedup_key = $1",
        )
        .bind(PLAYLIST)
        .fetch_one(&pool)
        .await?;
        assert_eq!(jobs, 1);
        let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM enqueue_commands")
            .fetch_one(&pool)
            .await?;
        assert!(
            (1..=2).contains(&commands),
            "{commands} enqueue commands reached core"
        );
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
        assert_eq!(status.queued_since, None);
        assert_eq!(status.error_code, None);
        assert_eq!(status.retry_after_seconds, 0);
        Ok(())
    }

    #[test]
    fn a_failing_read_is_named_only_once_its_streak_is_long() {
        let forbidden = Some("soundcloud_forbidden");
        assert_eq!(
            playlist_error_code("retry_wait", None, 5, forbidden),
            Some("forbidden")
        );
        assert_eq!(playlist_error_code("retry_wait", None, 2, forbidden), None);
        assert_eq!(
            playlist_error_code("retry_wait", None, 7, Some("soundcloud_rate_limited")),
            Some("unavailable")
        );
        assert_eq!(
            playlist_error_code("shadow_ready", None, 9, forbidden),
            None
        );
    }

    #[test]
    fn a_signed_out_owner_and_a_deleted_playlist_have_their_own_codes() {
        assert_eq!(
            playlist_error_code(
                "auth_required",
                None,
                1,
                Some("soundcloud_reauthorization_required")
            ),
            Some("auth_required")
        );
        assert_eq!(
            playlist_error_code(
                "conflict",
                Some("remote_not_found"),
                1,
                Some("soundcloud_playlist_not_found")
            ),
            Some("not_found")
        );
        assert_eq!(
            playlist_error_code("conflict", Some("legacy_order_only"), 9, None),
            None
        );
    }

    #[sqlx::test(migrations = false)]
    async fn status_reports_the_queued_read_and_a_retry_only_while_waiting_on_a_failure(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        test_schema::install(&pool).await?;
        test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
        let queued_at: DateTime<Utc> = sqlx::query_scalar(
            "INSERT INTO job_status (kind, dedup_key, state)
             VALUES ('playlists.observe_shadow', $1, 'queued')
             RETURNING updated_at",
        )
        .bind(PLAYLIST)
        .fetch_one(&pool)
        .await?;
        sqlx::query(
            "UPDATE playlist_membership_state
             SET sync_status = 'unhydrated',
                 next_reconcile_at = clock_timestamp() + interval '10 minutes'",
        )
        .execute(&pool)
        .await?;

        let unhydrated = membership_status(load_membership_status(&pool, PLAYLIST).await?);
        assert_eq!(unhydrated.queued_since, Some(queued_at));
        assert_eq!(unhydrated.retry_after_seconds, 0);
        assert_eq!(unhydrated.error_code, None);

        sqlx::query(
            "UPDATE playlist_membership_state
             SET sync_status = 'retry_wait',
                 reconcile_failure_streak = 5,
                 last_error = 'soundcloud_forbidden'",
        )
        .execute(&pool)
        .await?;
        sqlx::query("DELETE FROM job_status").execute(&pool).await?;

        let waiting = membership_status(load_membership_status(&pool, PLAYLIST).await?);
        assert!((599..=600).contains(&waiting.retry_after_seconds));
        assert_eq!(waiting.error_code, Some("forbidden"));
        assert_eq!(waiting.queued_since, None);
        Ok(())
    }
}
