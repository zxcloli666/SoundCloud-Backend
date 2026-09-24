use std::collections::HashSet;

use axum::http::StatusCode;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::playlists::edit::{self, MembershipRequest, Operation, TrackEdit};

pub const AWAITING_BASELINE: &str = "playlist_awaiting_baseline";
pub const LEGACY_RECONCILIATION_PENDING: &str = "playlist_legacy_reconciliation_pending";
pub const REVISION_CONFLICT: &str = "playlist_revision_conflict";
pub const UNKNOWN_TRACK: &str = "playlist_track_not_in_catalog";

const BASELINE_RETRY_SECONDS: i64 = 5;
const RECONCILE_DEBOUNCE_SECONDS: i64 = 30;

struct MembershipStateRow {
    baseline_generation: i64,
    baseline_observation_id: Option<Uuid>,
    projection_revision: i64,
    last_operation_sequence: i64,
    sync_status: String,
    has_legacy_intents: bool,
}

#[derive(Debug)]
pub struct JournalOutcome {
    pub appended: usize,
    pub projection_revision: i64,
}

pub struct PlaylistJournal {
    pool: PgPool,
}

impl PlaylistJournal {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn append(
        &self,
        playlist_urn: &str,
        actor_sc_user_id: &str,
        request: MembershipRequest,
        idempotency_key: Uuid,
    ) -> AppResult<JournalOutcome> {
        let mut transaction = self.pool.begin().await?;
        let result = self
            .append_in(
                &mut transaction,
                playlist_urn,
                actor_sc_user_id,
                request,
                idempotency_key,
            )
            .await;
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(|error| error.public_code() == AWAITING_BASELINE)
        {
            transaction.commit().await?;
        }
        result
    }

    pub(super) async fn append_in(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        playlist_urn: &str,
        actor_sc_user_id: &str,
        request: MembershipRequest,
        idempotency_key: Uuid,
    ) -> AppResult<JournalOutcome> {
        let state = sqlx::query_file_as!(
            MembershipStateRow,
            "queries/playlists/lock_membership_state.sql",
            playlist_urn
        )
        .fetch_optional(&mut **transaction)
        .await?;

        let Some(state) = state else {
            return Err(awaiting_baseline());
        };
        let owns = sqlx::query_file_scalar!(
            "queries/playlists/assert_owner.sql",
            actor_sc_user_id,
            playlist_urn,
            &crate::common::sc_ids::user_id_variants(actor_sc_user_id)
        )
        .fetch_one(&mut **transaction)
        .await?;
        if !owns {
            return Err(AppError::not_found("Playlist not found"));
        }
        let Some(base_observation_id) = state
            .baseline_observation_id
            .filter(|_| state.baseline_generation > 0 && state.sync_status != "unhydrated")
        else {
            sqlx::query_file!("queries/playlists/mark_reconcile_due.sql", playlist_urn)
                .execute(&mut **transaction)
                .await?;
            return Err(awaiting_baseline());
        };
        if state.has_legacy_intents || state.sync_status == "legacy_review" {
            return Err(AppError::coded(
                StatusCode::CONFLICT,
                LEGACY_RECONCILIATION_PENDING,
                "playlist membership is waiting for legacy reconciliation",
            ));
        }
        let replayed = sqlx::query_file_scalar!(
            "queries/playlists/count_replayed_operations.sql",
            playlist_urn,
            &[operation_key(idempotency_key, 0)]
        )
        .fetch_one(&mut **transaction)
        .await?;
        if replayed > 0 {
            return Ok(JournalOutcome {
                appended: 0,
                projection_revision: state.projection_revision,
            });
        }
        if let Some(expected) = request.expected_projection_revision
            && expected != state.projection_revision
        {
            return Err(AppError::coded(
                StatusCode::CONFLICT,
                REVISION_CONFLICT,
                "playlist membership changed since the submitted revision",
            ));
        }
        let edit = match request.edit {
            TrackEdit::Replace { track_ids } if request.expected_projection_revision.is_none() => {
                TrackEdit::Order { track_ids }
            }
            edit => edit,
        };

        let before =
            sqlx::query_file_scalar!("queries/playlists/load_projection.sql", playlist_urn)
                .fetch_all(&mut **transaction)
                .await?;
        let mut projection = before.clone();
        let operations = edit::derive(&edit, &projection)?;
        if operations.is_empty() {
            return Ok(JournalOutcome {
                appended: 0,
                projection_revision: state.projection_revision,
            });
        }

        let keys = operation_keys(idempotency_key, operations.len());
        assert_added_tracks_exist(transaction, &operations).await?;

        for (index, operation) in operations.iter().enumerate() {
            let sequence = state.last_operation_sequence + index as i64 + 1;
            let expected_revision = state.projection_revision + index as i64;
            sqlx::query_file!(
                "queries/playlists/append_operation.sql",
                Uuid::now_v7(),
                playlist_urn,
                sequence,
                actor_sc_user_id,
                keys[index],
                &operation.fingerprint(),
                state.baseline_generation,
                base_observation_id,
                expected_revision,
                operation.kind(),
                operation.track_id(),
                operation.left_anchor(),
                operation.right_anchor(),
                operation.boundary(),
                operation.ordered_track_ids()
            )
            .execute(&mut **transaction)
            .await?;
            operation.apply(&mut projection);
        }

        let projection_track_count = i32::try_from(projection.len())
            .map_err(|_| AppError::internal("playlist projection is too large"))?;
        write_projection(transaction, playlist_urn, &before, &projection).await?;
        record_playlist_adds(
            transaction,
            actor_sc_user_id,
            idempotency_key,
            &newly_added(&before, &projection),
        )
        .await?;

        let last_operation_sequence = state.last_operation_sequence + operations.len() as i64;
        let projection_revision = state.projection_revision + operations.len() as i64;
        sqlx::query_file!(
            "queries/playlists/advance_membership_state.sql",
            playlist_urn,
            last_operation_sequence,
            projection_revision,
            projection_track_count,
            RECONCILE_DEBOUNCE_SECONDS as f64
        )
        .execute(&mut **transaction)
        .await?;

        Ok(JournalOutcome {
            appended: operations.len(),
            projection_revision,
        })
    }
}

enum ProjectionWrite {
    Appended(Vec<String>),
    Removed(Vec<String>),
    Rewritten,
}

fn projection_write(before: &[String], after: &[String]) -> ProjectionWrite {
    if after.len() > before.len() && after.starts_with(before) {
        return ProjectionWrite::Appended(after[before.len()..].to_vec());
    }
    if after.len() < before.len() {
        let kept: HashSet<&str> = after.iter().map(String::as_str).collect();
        let surviving: Vec<&String> = before
            .iter()
            .filter(|track_id| kept.contains(track_id.as_str()))
            .collect();
        if surviving.len() == after.len()
            && surviving
                .iter()
                .zip(after)
                .all(|(survivor, expected)| *survivor == expected)
        {
            return ProjectionWrite::Removed(
                before
                    .iter()
                    .filter(|track_id| !kept.contains(track_id.as_str()))
                    .cloned()
                    .collect(),
            );
        }
    }
    ProjectionWrite::Rewritten
}

async fn write_projection(
    transaction: &mut Transaction<'_, Postgres>,
    playlist_urn: &str,
    before: &[String],
    after: &[String],
) -> AppResult<()> {
    match projection_write(before, after) {
        ProjectionWrite::Appended(added) => {
            sqlx::query_file!(
                "queries/playlists/append_projection_tracks.sql",
                playlist_urn,
                &added
            )
            .execute(&mut **transaction)
            .await?;
        }
        ProjectionWrite::Removed(dropped) => {
            sqlx::query_file!(
                "queries/playlists/delete_projection_tracks.sql",
                playlist_urn,
                &dropped
            )
            .execute(&mut **transaction)
            .await?;
        }
        ProjectionWrite::Rewritten => {
            sqlx::query_file!("queries/playlists/delete_projection.sql", playlist_urn)
                .execute(&mut **transaction)
                .await?;
            sqlx::query_file!(
                "queries/playlists/insert_projection.sql",
                playlist_urn,
                after
            )
            .execute(&mut **transaction)
            .await?;
        }
    }
    Ok(())
}

pub(super) fn newly_added(before: &[String], after: &[String]) -> Vec<String> {
    let members: HashSet<&str> = before.iter().map(String::as_str).collect();
    let mut added: Vec<String> = Vec::new();
    for track_id in after {
        if !members.contains(track_id.as_str()) && !added.contains(track_id) {
            added.push(track_id.clone());
        }
    }
    added
}

async fn record_playlist_adds(
    transaction: &mut Transaction<'_, Postgres>,
    actor_sc_user_id: &str,
    idempotency_key: Uuid,
    added: &[String],
) -> AppResult<()> {
    if added.is_empty() {
        return Ok(());
    }
    let event_ids: Vec<Uuid> = added
        .iter()
        .map(|track_id| Uuid::new_v5(&idempotency_key, track_id.as_bytes()))
        .collect();
    sqlx::query_file!(
        "queries/playlists/record_playlist_adds.sql",
        actor_sc_user_id,
        &event_ids,
        added,
        crate::modules::events::PLAYLIST_ADD_WEIGHT,
        &crate::common::sc_ids::user_id_variants(actor_sc_user_id)
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn assert_added_tracks_exist(
    transaction: &mut Transaction<'_, Postgres>,
    operations: &[Operation],
) -> AppResult<()> {
    let added: Vec<String> = operations
        .iter()
        .filter_map(|operation| operation.added_track().map(str::to_owned))
        .collect();
    if added.is_empty() {
        return Ok(());
    }
    let known = sqlx::query_file_scalar!("queries/playlists/count_catalog_tracks.sql", &added)
        .fetch_one(&mut **transaction)
        .await?;
    if usize::try_from(known).ok() != Some(added.len()) {
        return Err(AppError::coded(
            StatusCode::NOT_FOUND,
            UNKNOWN_TRACK,
            "a submitted track is not in the catalog yet",
        ));
    }
    Ok(())
}

fn operation_keys(idempotency_key: Uuid, count: usize) -> Vec<Uuid> {
    (0..count)
        .map(|index| operation_key(idempotency_key, index))
        .collect()
}

fn operation_key(idempotency_key: Uuid, index: usize) -> Uuid {
    Uuid::new_v5(&idempotency_key, &(index as u64).to_be_bytes())
}

fn awaiting_baseline() -> AppError {
    AppError::coded(
        StatusCode::CONFLICT,
        AWAITING_BASELINE,
        "playlist membership is waiting for its first SoundCloud observation",
    )
    .with_retry_after(BASELINE_RETRY_SECONDS)
}
