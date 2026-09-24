use std::collections::HashSet;

use backend_contracts::JobKind;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::classification::{MembershipRelation, membership_relation};
use super::fingerprint::membership_fingerprint;
use super::model::PlaylistSnapshot;
use super::reduce::{
    Boundary, Intent, Operation, Placement, Reduction, ReductionOutcome, Resolution,
    ResolvedOperation, committed_prefix, reduce,
};
use super::urn::PlaylistUrn;

pub const OPERATION_MALFORMED: &str = "operation_malformed";

pub struct PlaylistObserveRepository {
    pool: PgPool,
    membership_remote_apply: bool,
}

#[derive(Clone, Debug)]
pub struct ObservationCapture {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub job_generation: i64,
    pub playlist_urn: String,
    pub owner_id: String,
    pub reconcile_generation: i64,
    pub baseline_generation: i64,
    pub through_operation_sequence: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureResult {
    Ready,
    Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistResult {
    Applied,
    Superseded,
    Finished,
}

pub struct CapturedObservation {
    pub result: CaptureResult,
    pub capture: Option<ObservationCapture>,
}

pub struct FailureObservation {
    pub outcome: &'static str,
    pub run_decision: &'static str,
    pub state_status: &'static str,
    pub conflict_code: Option<&'static str>,
    pub retry_at: Option<DateTime<Utc>>,
    pub error_kind: String,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("playlist membership state is missing")]
    MissingState,

    #[error("playlist owner SoundCloud identity is unavailable")]
    MissingOwner,

    #[error("playlist observation snapshot is inconsistent")]
    InconsistentSnapshot,

    #[error("playlist track hydration payload is invalid: {0}")]
    InvalidHydration(#[from] serde_json::Error),

    #[error("playlist observation database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

struct ExistingRunRow {
    run_id: Uuid,
    reconcile_generation: i64,
    decision: String,
    state_reconcile_generation: i64,
    baseline_generation: i64,
    last_operation_sequence: i64,
    owner_sc_user_id: Option<String>,
}

struct AdvancedStateRow {
    reconcile_generation: i64,
    baseline_generation: i64,
    last_operation_sequence: i64,
    owner_sc_user_id: Option<String>,
}

struct LockedRunRow {
    decision: String,
    reconcile_generation: i64,
    captured_baseline_generation: i64,
    captured_through_operation_sequence: i64,
    state_reconcile_generation: i64,
    baseline_generation: i64,
    last_operation_sequence: i64,
    committed_operation_sequence: i64,
    owner_sc_user_id: Option<String>,
}

impl PlaylistObserveRepository {
    pub fn new(pool: PgPool, membership_remote_apply: bool) -> Self {
        Self {
            pool,
            membership_remote_apply,
        }
    }

    pub async fn claim_due(
        &self,
        batch: i64,
        owner_share: i64,
        claim_seconds: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let owners = i32::try_from(batch.div_euclid(owner_share.max(1)).max(1)).unwrap_or(i32::MAX);
        let urns = sqlx::query_file_scalar!(
            "queries/playlist_observe/claim_due_playlists.sql",
            owners,
            owner_share.max(1),
            batch,
            claim_seconds as f64
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(urns)
    }

    pub async fn capture(
        &self,
        urn: &PlaylistUrn,
        job_id: Uuid,
        job_generation: i64,
    ) -> Result<CapturedObservation, RepositoryError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query_file_scalar!("queries/playlist_observe/lock_membership.sql", urn.as_str())
            .fetch_optional(&mut *transaction)
            .await?;
        if sqlx::query_file_scalar!("queries/playlist_observe/is_deleted.sql", urn.as_str())
            .fetch_one(&mut *transaction)
            .await?
        {
            transaction.commit().await?;
            return Ok(finished_capture());
        }
        let existing = sqlx::query_file_as!(
            ExistingRunRow,
            "queries/playlist_observe/load_run_for_capture.sql",
            urn.as_str(),
            job_id,
            job_generation
        )
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(existing) = existing {
            if existing.state_reconcile_generation != existing.reconcile_generation {
                mark_superseded(
                    &mut transaction,
                    existing.run_id,
                    None,
                    "a newer playlist observation replaced this run",
                )
                .await?;
                transaction.commit().await?;
                return Ok(finished_capture());
            }
            if !matches!(
                existing.decision.as_str(),
                "started" | "auth_required" | "retry_wait" | "incomplete"
            ) {
                transaction.commit().await?;
                return Ok(finished_capture());
            }
            sqlx::query_file!(
                "queries/playlist_observe/restart_run.sql",
                existing.run_id,
                existing.baseline_generation,
                existing.last_operation_sequence
            )
            .execute(&mut *transaction)
            .await?;
            let owner_id = required_owner(existing.owner_sc_user_id)?;
            let capture = ObservationCapture {
                run_id: existing.run_id,
                job_id,
                job_generation,
                playlist_urn: urn.as_str().to_owned(),
                owner_id,
                reconcile_generation: existing.reconcile_generation,
                baseline_generation: existing.baseline_generation,
                through_operation_sequence: existing.last_operation_sequence,
            };
            transaction.commit().await?;
            return Ok(ready_capture(capture));
        }

        let state = sqlx::query_file_as!(
            AdvancedStateRow,
            "queries/playlist_observe/advance_state.sql",
            urn.as_str()
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(RepositoryError::MissingState)?;
        let run_id = sqlx::query_file_scalar!(
            "queries/playlist_observe/insert_run.sql",
            urn.as_str(),
            state.reconcile_generation,
            job_id,
            job_generation,
            state.baseline_generation,
            state.last_operation_sequence
        )
        .fetch_one(&mut *transaction)
        .await?;
        let capture = ObservationCapture {
            run_id,
            job_id,
            job_generation,
            playlist_urn: urn.as_str().to_owned(),
            owner_id: required_owner(state.owner_sc_user_id)?,
            reconcile_generation: state.reconcile_generation,
            baseline_generation: state.baseline_generation,
            through_operation_sequence: state.last_operation_sequence,
        };
        transaction.commit().await?;
        Ok(ready_capture(capture))
    }

    pub async fn persist_success(
        &self,
        capture: &ObservationCapture,
        snapshot: &PlaylistSnapshot,
        metadata_observation: catalog_ingest::Observation,
    ) -> Result<PersistResult, RepositoryError> {
        if snapshot.owner_id != capture.owner_id
            || snapshot.track_count < 0
            || usize::try_from(snapshot.track_count).ok() != Some(snapshot.track_ids.len())
            || !hydration_matches_snapshot(snapshot)
        {
            return Err(RepositoryError::InconsistentSnapshot);
        }
        let mut transaction = self.pool.begin().await?;
        let locked = lock_run(&mut transaction, capture).await?;
        if locked.decision != "started" {
            transaction.commit().await?;
            return Ok(PersistResult::Finished);
        }
        let fingerprint = membership_fingerprint(&snapshot.track_ids);
        let snapshot_id = store_snapshot(
            &mut transaction,
            &capture.playlist_urn,
            &snapshot.track_ids,
            &fingerprint,
        )
        .await?;
        let observation_id = sqlx::query_file_scalar!(
            "queries/playlist_observe/insert_complete_observation.sql",
            &capture.playlist_urn,
            snapshot_id,
            snapshot.track_count,
            snapshot.remote_last_modified,
            snapshot.observed_at
        )
        .fetch_one(&mut *transaction)
        .await?;
        if !fence_matches(&locked, capture) {
            mark_superseded(
                &mut transaction,
                capture.run_id,
                Some(observation_id),
                "playlist state changed while SoundCloud was being observed",
            )
            .await?;
            transaction.commit().await?;
            return Ok(PersistResult::Superseded);
        }

        let catalog_complete =
            hydrate_catalog(&mut transaction, snapshot, metadata_observation).await?;

        let local_track_ids = sqlx::query_file_scalar!(
            "queries/playlist_observe/load_projection.sql",
            &capture.playlist_urn
        )
        .fetch_all(&mut *transaction)
        .await?;
        let has_legacy_intents = sqlx::query_file_scalar!(
            "queries/playlist_observe/has_legacy_intents.sql",
            &capture.playlist_urn
        )
        .fetch_one(&mut *transaction)
        .await?;
        let is_legacy = has_legacy_intents;
        let relation = membership_relation(&local_track_ids, &snapshot.track_ids);
        if has_legacy_intents {
            let local_fingerprint = membership_fingerprint(&local_track_ids);
            let local_snapshot_id = store_snapshot(
                &mut transaction,
                &capture.playlist_urn,
                &local_track_ids,
                &local_fingerprint,
            )
            .await?;
            sqlx::query_file!(
                "queries/playlist_observe/classify_legacy_intents.sql",
                &capture.playlist_urn,
                local_snapshot_id,
                relation.as_str()
            )
            .execute(&mut *transaction)
            .await?;
        }
        let pending_operations =
            locked.last_operation_sequence > locked.committed_operation_sequence;
        let outcome = if pending_operations && catalog_complete && !is_legacy {
            self.reduce_pending_operations(&mut transaction, capture, &locked, snapshot)
                .await?
        } else {
            plain_reconciliation(
                reconciliation_decision(is_legacy, pending_operations, catalog_complete, relation),
                snapshot,
                &fingerprint,
                locked.committed_operation_sequence,
            )
        };
        let projection_changed = outcome
            .projection
            .as_ref()
            .is_some_and(|target| *target != local_track_ids);
        if projection_changed {
            let Some(target) = outcome.projection.as_ref() else {
                return Err(RepositoryError::InconsistentSnapshot);
            };
            sqlx::query_file!(
                "queries/playlist_observe/delete_projection.sql",
                &capture.playlist_urn
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query_file!(
                "queries/playlist_observe/insert_projection.sql",
                &capture.playlist_urn,
                target
            )
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query_file!(
            "queries/playlist_observe/update_playlist_metadata.sql",
            &capture.playlist_urn,
            snapshot.track_count
        )
        .execute(&mut *transaction)
        .await?;
        let projection_length = match outcome.projection.as_ref() {
            Some(target) if projection_changed => target.len(),
            _ => local_track_ids.len(),
        };
        let projection_count =
            i32::try_from(projection_length).map_err(|_| RepositoryError::InconsistentSnapshot)?;
        resolve_operations(&mut transaction, &capture.playlist_urn, &outcome.operations).await?;
        let state_updated = sqlx::query_file!(
            "queries/playlist_observe/complete_state.sql",
            &capture.playlist_urn,
            observation_id,
            projection_changed,
            projection_count,
            outcome.state_status,
            outcome.conflict_code,
            outcome.candidate_fingerprint.as_deref(),
            outcome.reason.as_deref(),
            capture.reconcile_generation,
            outcome.committed_through_sequence,
            &fingerprint
        )
        .execute(&mut *transaction)
        .await?;
        if state_updated.rows_affected() != 1 {
            return Err(RepositoryError::InconsistentSnapshot);
        }
        self.enqueue_membership_apply(&mut transaction, capture, &outcome)
            .await?;
        let run_updated = sqlx::query_file!(
            "queries/playlist_observe/complete_run.sql",
            capture.run_id,
            observation_id,
            &fingerprint,
            outcome.run_decision,
            outcome.reason.as_deref()
        )
        .execute(&mut *transaction)
        .await?;
        if run_updated.rows_affected() != 1 {
            return Err(RepositoryError::InconsistentSnapshot);
        }
        transaction.commit().await?;
        Ok(PersistResult::Applied)
    }

    async fn enqueue_membership_apply(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        capture: &ObservationCapture,
        outcome: &Reconciliation,
    ) -> Result<(), RepositoryError> {
        if !self.membership_remote_apply || outcome.state_status != "shadow_ready" {
            return Ok(());
        }
        let (Some(candidate), Some(candidate_fingerprint)) =
            (&outcome.projection, &outcome.candidate_fingerprint)
        else {
            return Ok(());
        };
        sqlx::query_file!(
            "queries/playlist_observe/enqueue_membership_apply.sql",
            &capture.playlist_urn,
            &capture.owner_id,
            candidate,
            candidate_fingerprint.as_slice(),
            capture.reconcile_generation
        )
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    async fn reduce_pending_operations(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        capture: &ObservationCapture,
        locked: &LockedRunRow,
        snapshot: &PlaylistSnapshot,
    ) -> Result<Reconciliation, RepositoryError> {
        let rows = sqlx::query_file_as!(
            PendingOperationRow,
            "queries/playlist_observe/load_pending_operations.sql",
            &capture.playlist_urn,
            capture.through_operation_sequence
        )
        .fetch_all(&mut **transaction)
        .await?;
        let mut operations = Vec::with_capacity(rows.len());
        let mut malformed = Vec::new();
        for row in &rows {
            match row.intent() {
                Some(intent) => operations.push(Operation {
                    operation_id: row.operation_id,
                    sequence: row.sequence,
                    intent,
                }),
                None => malformed.push(ResolvedOperation {
                    operation_id: row.operation_id,
                    sequence: row.sequence,
                    resolution: Resolution::Conflict(OPERATION_MALFORMED),
                }),
            }
        }
        let reduction = reduce(
            &snapshot.track_ids,
            &operations,
            locked.committed_operation_sequence,
        );
        let baseline = sqlx::query_file_scalar!(
            "queries/playlist_observe/load_baseline_tracks.sql",
            &capture.playlist_urn
        )
        .fetch_all(&mut **transaction)
        .await?;
        let remote_moved = membership_relation(&baseline, &snapshot.track_ids);
        let candidate_complete = if reduction.candidate == snapshot.track_ids {
            true
        } else {
            schedule_missing_tracks(transaction, &reduction.candidate).await?
        };
        Ok(reduced_reconciliation(
            reduction,
            malformed,
            remote_moved,
            candidate_complete,
            locked.committed_operation_sequence,
        ))
    }

    pub async fn persist_failure(
        &self,
        capture: &ObservationCapture,
        failure: &FailureObservation,
    ) -> Result<PersistResult, RepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let locked = lock_run(&mut transaction, capture).await?;
        if locked.decision != "started" {
            transaction.commit().await?;
            return Ok(PersistResult::Finished);
        }
        let observation_id = sqlx::query_file_scalar!(
            "queries/playlist_observe/insert_failure_observation.sql",
            &capture.playlist_urn,
            failure.outcome,
            failure.observed_at,
            failure.retry_at,
            &failure.error_kind
        )
        .fetch_one(&mut *transaction)
        .await?;
        if !fence_matches(&locked, capture) {
            mark_superseded(
                &mut transaction,
                capture.run_id,
                Some(observation_id),
                "playlist state changed while SoundCloud was being observed",
            )
            .await?;
            transaction.commit().await?;
            return Ok(PersistResult::Superseded);
        }
        let state_updated = sqlx::query_file!(
            "queries/playlist_observe/complete_failure_state.sql",
            &capture.playlist_urn,
            observation_id,
            failure.state_status,
            failure.conflict_code,
            failure.retry_at,
            &failure.error_kind,
            capture.reconcile_generation
        )
        .execute(&mut *transaction)
        .await?;
        if state_updated.rows_affected() != 1 {
            return Err(RepositoryError::InconsistentSnapshot);
        }
        let run_updated = sqlx::query_file!(
            "queries/playlist_observe/complete_run.sql",
            capture.run_id,
            observation_id,
            Option::<Vec<u8>>::None,
            failure.run_decision,
            &failure.error_kind
        )
        .execute(&mut *transaction)
        .await?;
        if run_updated.rows_affected() != 1 {
            return Err(RepositoryError::InconsistentSnapshot);
        }
        transaction.commit().await?;
        Ok(PersistResult::Applied)
    }
}

struct ReconciliationDecision {
    run_decision: &'static str,
    state_status: &'static str,
    conflict_code: Option<&'static str>,
    reason: Option<&'static str>,
    replace_projection: bool,
}

struct Reconciliation {
    run_decision: &'static str,
    state_status: &'static str,
    conflict_code: Option<&'static str>,
    reason: Option<String>,
    projection: Option<Vec<String>>,
    candidate_fingerprint: Option<Vec<u8>>,
    committed_through_sequence: i64,
    operations: Vec<ResolvedOperation>,
}

struct PendingOperationRow {
    operation_id: Uuid,
    sequence: i64,
    kind: String,
    track_id: Option<String>,
    left_anchor_track_id: Option<String>,
    right_anchor_track_id: Option<String>,
    boundary: Option<String>,
    ordered_track_ids: Option<Vec<String>>,
}

impl PendingOperationRow {
    fn intent(&self) -> Option<Intent> {
        match self.kind.as_str() {
            "add" => Some(Intent::Add {
                track_id: self.track_id.clone()?,
                placement: self.placement()?,
            }),
            "remove" => Some(Intent::Remove {
                track_id: self.track_id.clone()?,
            }),
            "move" => Some(Intent::Move {
                track_id: self.track_id.clone()?,
                placement: self.placement()?,
            }),
            "reorder" => {
                let ordered_track_ids = self.ordered_track_ids.clone()?;
                (!ordered_track_ids.is_empty()).then_some(Intent::Reorder { ordered_track_ids })
            }
            _ => None,
        }
    }

    fn placement(&self) -> Option<Placement> {
        match self.boundary.as_deref() {
            Some("front") => Some(Placement::Boundary(Boundary::Front)),
            Some("back") => Some(Placement::Boundary(Boundary::Back)),
            Some(_) => None,
            None => {
                let left = self.left_anchor_track_id.clone();
                let right = self.right_anchor_track_id.clone();
                (left.is_some() || right.is_some()).then_some(Placement::Anchored { left, right })
            }
        }
    }
}

fn plain_reconciliation(
    decision: ReconciliationDecision,
    snapshot: &PlaylistSnapshot,
    fingerprint: &[u8],
    committed_through: i64,
) -> Reconciliation {
    Reconciliation {
        run_decision: decision.run_decision,
        state_status: decision.state_status,
        conflict_code: decision.conflict_code,
        reason: decision.reason.map(str::to_owned),
        projection: decision
            .replace_projection
            .then(|| snapshot.track_ids.clone()),
        candidate_fingerprint: (decision.state_status != "clean").then(|| fingerprint.to_vec()),
        committed_through_sequence: committed_through,
        operations: Vec::new(),
    }
}

fn reduced_reconciliation(
    reduction: Reduction,
    malformed: Vec<ResolvedOperation>,
    remote_moved: MembershipRelation,
    candidate_complete: bool,
    committed_through: i64,
) -> Reconciliation {
    let Reduction {
        candidate,
        mut operations,
        outcome,
        anchors_lost,
        ..
    } = reduction;
    operations.extend(malformed);
    operations.sort_by_key(|operation| operation.sequence);
    let candidate_fingerprint = membership_fingerprint(&candidate);

    if !candidate_complete {
        return Reconciliation {
            run_decision: "incomplete",
            state_status: "conflict",
            conflict_code: Some("catalog_incomplete"),
            reason: Some(
                "the rebased playlist still contains a track without a durable catalog row"
                    .to_owned(),
            ),
            projection: None,
            candidate_fingerprint: Some(candidate_fingerprint),
            committed_through_sequence: committed_through,
            operations: Vec::new(),
        };
    }

    let conflict = operations
        .iter()
        .find_map(|operation| operation.resolution.conflict_code());
    let (run_decision, state_status, conflict_code) = match (conflict, outcome) {
        (Some(code), _) => ("conflict", "conflict", Some(code)),
        (None, ReductionOutcome::Converged) => ("clean", "clean", None),
        (None, _) => ("shadow_ready", "shadow_ready", None),
    };
    let pending = operations
        .iter()
        .filter(|operation| operation.resolution == Resolution::Pending)
        .count();
    Reconciliation {
        run_decision,
        state_status,
        conflict_code,
        reason: reduction_reason(state_status, pending, remote_moved, anchors_lost),
        projection: Some(candidate),
        candidate_fingerprint: (state_status != "clean").then_some(candidate_fingerprint),
        committed_through_sequence: committed_prefix(&operations, committed_through),
        operations,
    }
}

fn reduction_reason(
    state_status: &str,
    pending: usize,
    remote_moved: MembershipRelation,
    anchors_lost: bool,
) -> Option<String> {
    if state_status == "clean" {
        return None;
    }
    let mut reason = if state_status == "conflict" {
        format!(
            "a local operation could not be replayed onto the fresh remote playlist; remote membership is {}",
            remote_moved.as_str()
        )
    } else {
        format!(
            "{pending} local operations rebased onto the fresh remote playlist; remote membership is {}",
            remote_moved.as_str()
        )
    };
    if anchors_lost {
        reason.push_str("; an operation anchor was lost and its track kept a boundary position");
    }
    Some(reason)
}

async fn resolve_operations(
    transaction: &mut Transaction<'_, Postgres>,
    playlist_urn: &str,
    operations: &[ResolvedOperation],
) -> Result<(), RepositoryError> {
    let terminal: Vec<&ResolvedOperation> = operations
        .iter()
        .filter(|operation| operation.resolution.is_terminal())
        .collect();
    if terminal.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = terminal
        .iter()
        .map(|operation| operation.operation_id)
        .collect();
    let outcomes: Vec<String> = terminal
        .iter()
        .map(|operation| operation.resolution.outcome().to_owned())
        .collect();
    let conflict_codes: Vec<Option<String>> = terminal
        .iter()
        .map(|operation| operation.resolution.conflict_code().map(str::to_owned))
        .collect();
    sqlx::query_file!(
        "queries/playlist_observe/resolve_operations.sql",
        playlist_urn,
        &ids,
        &outcomes,
        &conflict_codes as &[Option<String>]
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn reconciliation_decision(
    is_legacy: bool,
    pending_operations: bool,
    catalog_complete: bool,
    relation: MembershipRelation,
) -> ReconciliationDecision {
    if !catalog_complete {
        return ReconciliationDecision {
            run_decision: "incomplete",
            state_status: "conflict",
            conflict_code: Some("catalog_incomplete"),
            reason: Some("not every observed track has a durable catalog row"),
            replace_projection: false,
        };
    }
    if pending_operations {
        return ReconciliationDecision {
            run_decision: "shadow_ready",
            state_status: "shadow_ready",
            conflict_code: None,
            reason: Some("local operations require a later reducer"),
            replace_projection: false,
        };
    }
    if !is_legacy {
        return ReconciliationDecision {
            run_decision: "clean",
            state_status: "clean",
            conflict_code: None,
            reason: None,
            replace_projection: true,
        };
    }
    match relation {
        MembershipRelation::Equal => ReconciliationDecision {
            run_decision: "legacy_equal",
            state_status: "clean",
            conflict_code: None,
            reason: None,
            replace_projection: false,
        },
        MembershipRelation::RemoteSuperset => ReconciliationDecision {
            run_decision: "remote_superset",
            state_status: "conflict",
            conflict_code: Some("legacy_remote_superset"),
            reason: Some("the remote playlist safely extends the local legacy projection"),
            replace_projection: true,
        },
        MembershipRelation::LocalSuperset => ReconciliationDecision {
            run_decision: "local_superset",
            state_status: "conflict",
            conflict_code: Some("legacy_local_superset"),
            reason: Some("the local legacy projection contains tracks missing remotely"),
            replace_projection: false,
        },
        MembershipRelation::OrderOnly => ReconciliationDecision {
            run_decision: "order_only",
            state_status: "conflict",
            conflict_code: Some("legacy_order_only"),
            reason: Some("the local and remote playlist orders differ"),
            replace_projection: false,
        },
        MembershipRelation::Diverged => ReconciliationDecision {
            run_decision: "membership_diverged",
            state_status: "conflict",
            conflict_code: Some("legacy_membership_diverged"),
            reason: Some("the local and remote playlist memberships diverged"),
            replace_projection: false,
        },
    }
}

fn hydration_matches_snapshot(snapshot: &PlaylistSnapshot) -> bool {
    let track_ids = snapshot
        .track_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let hydrated_ids = snapshot
        .hydrated_tracks
        .iter()
        .map(|track| track.sc_track_id.as_str())
        .collect::<HashSet<_>>();
    hydrated_ids.len() == snapshot.hydrated_tracks.len()
        && hydrated_ids.is_subset(&track_ids)
        && snapshot.hydrated_tracks.iter().all(|track| {
            track.urn == format!("soundcloud:tracks:{}", track.sc_track_id)
                && !track.title.trim().is_empty()
        })
}

async fn hydrate_catalog(
    transaction: &mut Transaction<'_, Postgres>,
    snapshot: &PlaylistSnapshot,
    metadata_observation: catalog_ingest::Observation,
) -> Result<bool, RepositoryError> {
    if !snapshot.hydrated_tracks.is_empty() {
        let payload = serde_json::to_value(&snapshot.hydrated_tracks)?;
        sqlx::query_file!(
            "queries/playlist_observe/upsert_tracks.sql",
            payload,
            metadata_observation.sequence()
        )
        .execute(&mut **transaction)
        .await?;
    }
    schedule_missing_tracks(transaction, &snapshot.track_ids).await
}

async fn schedule_missing_tracks(
    transaction: &mut Transaction<'_, Postgres>,
    track_ids: &[String],
) -> Result<bool, RepositoryError> {
    if track_ids.is_empty() {
        return Ok(true);
    }
    let missing = sqlx::query_file_scalar!(
        "queries/playlist_observe/missing_catalog_tracks.sql",
        track_ids
    )
    .fetch_all(&mut **transaction)
    .await?;
    if missing.is_empty() {
        return Ok(true);
    }
    sqlx::query_file!(
        "queries/playlist_observe/enqueue_missing_tracks.sql",
        JobKind::CatalogRefresh.lane().as_str(),
        &missing
    )
    .execute(&mut **transaction)
    .await?;
    Ok(false)
}

async fn lock_run(
    transaction: &mut Transaction<'_, Postgres>,
    capture: &ObservationCapture,
) -> Result<LockedRunRow, RepositoryError> {
    sqlx::query_file_scalar!(
        "queries/playlist_observe/lock_membership.sql",
        &capture.playlist_urn
    )
    .fetch_optional(&mut **transaction)
    .await?;
    sqlx::query_file_as!(
        LockedRunRow,
        "queries/playlist_observe/lock_run_for_persist.sql",
        &capture.playlist_urn,
        capture.run_id,
        capture.job_id,
        capture.job_generation
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(RepositoryError::MissingState)
}

async fn store_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    playlist_urn: &str,
    track_ids: &[String],
    fingerprint: &[u8],
) -> Result<Uuid, RepositoryError> {
    let track_count =
        i32::try_from(track_ids.len()).map_err(|_| RepositoryError::InconsistentSnapshot)?;
    let snapshot_id = sqlx::query_file_scalar!(
        "queries/playlist_observe/upsert_snapshot.sql",
        playlist_urn,
        fingerprint,
        track_count
    )
    .fetch_one(&mut **transaction)
    .await?;
    sqlx::query_file!(
        "queries/playlist_observe/insert_snapshot_tracks.sql",
        snapshot_id,
        track_ids
    )
    .execute(&mut **transaction)
    .await?;
    let stored = sqlx::query_file_scalar!(
        "queries/playlist_observe/load_snapshot_tracks.sql",
        snapshot_id
    )
    .fetch_all(&mut **transaction)
    .await?;
    if stored != track_ids {
        return Err(RepositoryError::InconsistentSnapshot);
    }
    Ok(snapshot_id)
}

async fn mark_superseded(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    observation_id: Option<Uuid>,
    reason: &str,
) -> Result<(), RepositoryError> {
    sqlx::query_file!(
        "queries/playlist_observe/mark_run_superseded.sql",
        run_id,
        observation_id,
        reason
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn fence_matches(locked: &LockedRunRow, capture: &ObservationCapture) -> bool {
    locked.reconcile_generation == capture.reconcile_generation
        && locked.state_reconcile_generation == capture.reconcile_generation
        && locked.captured_baseline_generation == capture.baseline_generation
        && locked.baseline_generation == capture.baseline_generation
        && locked.captured_through_operation_sequence == capture.through_operation_sequence
        && locked.last_operation_sequence == capture.through_operation_sequence
        && locked
            .owner_sc_user_id
            .as_deref()
            .and_then(normalize_owner)
            .is_some_and(|owner| owner == capture.owner_id)
}

fn required_owner(owner: Option<String>) -> Result<String, RepositoryError> {
    owner
        .as_deref()
        .and_then(normalize_owner)
        .map(str::to_owned)
        .ok_or(RepositoryError::MissingOwner)
}

fn normalize_owner(value: &str) -> Option<&str> {
    let value = value.strip_prefix("soundcloud:users:").unwrap_or(value);
    value
        .parse::<u64>()
        .is_ok_and(|id| id > 0 && id.to_string() == value)
        .then_some(value)
}

fn ready_capture(capture: ObservationCapture) -> CapturedObservation {
    CapturedObservation {
        result: CaptureResult::Ready,
        capture: Some(capture),
    }
}

fn finished_capture() -> CapturedObservation {
    CapturedObservation {
        result: CaptureResult::Finished,
        capture: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_observation_replaces_the_projection() {
        let decision = reconciliation_decision(false, false, true, MembershipRelation::Diverged);

        assert_eq!(decision.run_decision, "clean");
        assert!(decision.replace_projection);
    }

    #[test]
    fn pending_operations_are_never_overwritten() {
        let decision =
            reconciliation_decision(true, true, true, MembershipRelation::RemoteSuperset);

        assert_eq!(decision.state_status, "shadow_ready");
        assert!(!decision.replace_projection);
    }

    #[test]
    fn legacy_remote_superset_is_the_only_non_equal_automatic_repair() {
        let safe = reconciliation_decision(true, false, true, MembershipRelation::RemoteSuperset);
        let unsafe_decision =
            reconciliation_decision(true, false, true, MembershipRelation::LocalSuperset);

        assert!(safe.replace_projection);
        assert_eq!(safe.state_status, "conflict");
        assert_eq!(safe.conflict_code, Some("legacy_remote_superset"));
        assert!(!unsafe_decision.replace_projection);
        assert_eq!(unsafe_decision.state_status, "conflict");
    }

    #[test]
    fn incomplete_catalog_cannot_advance_projection_count() {
        let decision =
            reconciliation_decision(false, false, false, MembershipRelation::RemoteSuperset);

        assert_eq!(decision.conflict_code, Some("catalog_incomplete"));
        assert!(!decision.replace_projection);
    }
}
