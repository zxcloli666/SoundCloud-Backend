mod musicbrainz_names;
mod renormalize;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind, Versioned};
use catalog_sources::MbClient;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::AdminMaintenanceConfig;
use crate::queue::{JobError, JobRepository, JobResult, LeasedJob, NewJob, QueueError};

pub const CATALOG_RENORMALIZE: &str = "catalog_renormalize";
pub const MUSICBRAINZ_NAMES: &str = "musicbrainz_names";

const SLICE_DEADLINE: Duration = Duration::from_secs(180);
const CONTINUATION_DELAY: Duration = Duration::from_secs(1);
const MAX_ATTEMPTS: i16 = 8;

#[derive(Debug, Clone, Copy, Default)]
pub struct SliceProgress {
    pub scanned: i64,
    pub changed: i64,
    pub merged: i64,
    pub skipped: i64,
}

#[derive(Debug, Clone, Copy)]
pub enum PhaseOutcome {
    Advanced,
    Finished,
}

#[derive(Debug, Clone)]
struct RunState {
    run_id: Uuid,
    phase: String,
    cursor_uuid: Option<Uuid>,
    cursor_text: Option<String>,
}

pub struct AdminMaintenanceHandler {
    pool: PgPool,
    queue: JobRepository,
    musicbrainz: Arc<MbClient>,
    scan_batch: i64,
}

impl AdminMaintenanceHandler {
    pub fn new(pool: PgPool, musicbrainz: Arc<MbClient>, config: AdminMaintenanceConfig) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "admin-maintenance".to_owned()),
            pool,
            musicbrainz,
            scan_batch: config.scan_batch,
        }
    }

    pub async fn renormalize_catalog(
        &self,
        job: &LeasedJob,
        payload: AdminMaintenancePayload,
    ) -> JobResult {
        self.run(job, payload, CATALOG_RENORMALIZE).await
    }

    pub async fn reconcile_musicbrainz_names(
        &self,
        job: &LeasedJob,
        payload: AdminMaintenancePayload,
    ) -> JobResult {
        self.run(job, payload, MUSICBRAINZ_NAMES).await
    }

    async fn run(
        &self,
        job: &LeasedJob,
        payload: AdminMaintenancePayload,
        kind: &'static str,
    ) -> JobResult {
        if job.dedup_key.as_deref() != Some(kind) {
            return Err(JobError::permanent(anyhow::anyhow!(
                "admin maintenance job identity does not match its payload"
            )));
        }
        let first_phase = self.first_phase(kind);
        sqlx::query_file!(
            "queries/admin_maintenance/ensure_run.sql",
            kind,
            payload.run_id,
            first_phase
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let Some(state) = self.load_run(kind).await? else {
            return Ok(());
        };
        if state.run_id != payload.run_id {
            return Ok(());
        }

        let outcome = tokio::time::timeout(SLICE_DEADLINE, self.execute_slice(kind, &state)).await;
        let outcome = match outcome {
            Ok(outcome) => outcome?,
            Err(_) => PhaseOutcome::Advanced,
        };

        match outcome {
            PhaseOutcome::Advanced => self.enqueue_continuation(kind, payload.run_id).await,
            PhaseOutcome::Finished => {
                sqlx::query_file!(
                    "queries/admin_maintenance/complete_run.sql",
                    kind,
                    payload.run_id
                )
                .execute(&self.pool)
                .await
                .map_err(JobError::retryable)?;
                Ok(())
            }
        }
    }

    fn first_phase(&self, kind: &str) -> &'static str {
        if kind == MUSICBRAINZ_NAMES {
            musicbrainz_names::FIRST_PHASE
        } else {
            renormalize::FIRST_PHASE
        }
    }

    async fn execute_slice(&self, kind: &str, state: &RunState) -> JobResult<PhaseOutcome> {
        if kind == MUSICBRAINZ_NAMES {
            musicbrainz_names::execute(self, state).await
        } else {
            renormalize::execute(self, state).await
        }
    }

    async fn load_run(&self, kind: &str) -> JobResult<Option<RunState>> {
        let row = sqlx::query_file!("queries/admin_maintenance/load_run.sql", kind)
            .fetch_optional(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        let Some(row) = row.filter(|row| row.status == "running") else {
            return Ok(None);
        };
        Ok(Some(RunState {
            run_id: row.run_id,
            phase: row.phase,
            cursor_uuid: row.cursor_uuid,
            cursor_text: row.cursor_text,
        }))
    }

    async fn save_progress(
        &self,
        kind: &str,
        state: &RunState,
        cursor_uuid: Option<Uuid>,
        cursor_text: Option<&str>,
        progress: SliceProgress,
    ) -> JobResult {
        sqlx::query_file!(
            "queries/admin_maintenance/save_progress.sql",
            kind,
            state.run_id,
            &state.phase,
            cursor_uuid,
            cursor_text,
            progress.scanned,
            progress.changed,
            progress.merged,
            progress.skipped
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn advance_phase(&self, kind: &str, state: &RunState, phase: &str) -> JobResult {
        sqlx::query_file!(
            "queries/admin_maintenance/advance_phase.sql",
            kind,
            state.run_id,
            phase
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn enqueue_continuation(&self, kind: &'static str, run_id: Uuid) -> JobResult {
        let job = NewJob {
            id: Uuid::now_v7(),
            kind: continuation_kind(kind),
            dedup_key: Some(kind.to_owned()),
            payload: serde_json::to_value(Versioned::V1(AdminMaintenancePayload { run_id }))
                .map_err(JobError::permanent)?,
            priority: 20,
            max_attempts: MAX_ATTEMPTS,
            available_at: chrono::Utc::now()
                + chrono::Duration::from_std(CONTINUATION_DELAY).unwrap_or_default(),
        };
        self.queue
            .enqueue(&job)
            .await
            .map(|_| ())
            .map_err(queue_error)
    }
}

fn continuation_kind(kind: &str) -> JobKind {
    if kind == MUSICBRAINZ_NAMES {
        JobKind::AdminMusicBrainzNames
    } else {
        JobKind::AdminCatalogRenormalize
    }
}

fn queue_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}
