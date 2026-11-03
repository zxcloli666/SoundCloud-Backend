use axum::http::StatusCode;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::playlists::edit::{self, MembershipRequest, Operation, TrackEdit};

pub const REVISION_CONFLICT: &str = "playlist_revision_conflict";
pub const UNKNOWN_TRACK: &str = "playlist_track_not_in_catalog";
const NOT_FOUND: &str = "playlist_not_found";

struct ReplayedOperation {
    outcome: String,
    conflict_code: Option<String>,
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
        crate::db::retry_conflicts(|| async {
            let mut transaction = self.pool.begin().await?;
            let outcome = self
                .append_in(
                    &mut transaction,
                    playlist_urn,
                    actor_sc_user_id,
                    request.clone(),
                    idempotency_key,
                )
                .await?;
            transaction.commit().await?;
            Ok(outcome)
        })
        .await
    }

    pub(super) async fn append_in(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        playlist_urn: &str,
        actor_sc_user_id: &str,
        request: MembershipRequest,
        idempotency_key: Uuid,
    ) -> AppResult<JournalOutcome> {
        let projection_revision =
            sqlx::query_file_scalar!("queries/playlists/lock_membership_state.sql", playlist_urn)
                .fetch_optional(&mut **transaction)
                .await?
                .unwrap_or(0);
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
        let replayed = sqlx::query_file_as!(
            ReplayedOperation,
            "queries/playlists/replayed_operation.sql",
            playlist_urn,
            operation_key(idempotency_key, 0)
        )
        .fetch_optional(&mut **transaction)
        .await?;
        if let Some(replayed) = replayed {
            if replayed.outcome == "conflict"
                && let Some(refusal) = replayed.conflict_code.as_deref().and_then(refused)
            {
                return Err(refusal);
            }
            return Ok(JournalOutcome {
                appended: 0,
                projection_revision,
            });
        }
        if let Some(expected) = request.expected_projection_revision
            && expected != projection_revision
        {
            return Err(revision_conflict());
        }
        let edit = match request.edit {
            TrackEdit::Replace { track_ids } if request.expected_projection_revision.is_none() => {
                TrackEdit::Order { track_ids }
            }
            edit => edit,
        };

        let projection =
            sqlx::query_file_scalar!("queries/playlists/load_projection.sql", playlist_urn)
                .fetch_all(&mut **transaction)
                .await?;
        let operations = edit::derive(&edit, &projection)?;
        if operations.is_empty() {
            return Ok(JournalOutcome {
                appended: 0,
                projection_revision,
            });
        }
        assert_added_tracks_exist(transaction, &operations).await?;

        let command = wt::MembershipOp {
            playlist_urn: playlist_urn.to_owned(),
            actor: actor_sc_user_id.to_owned(),
            variants: crate::common::sc_ids::user_id_variants(actor_sc_user_id),
            expected_projection_revision: request.expected_projection_revision,
            operations: operations
                .iter()
                .enumerate()
                .map(|(index, operation)| journal_entry(idempotency_key, index, operation))
                .collect(),
            at: wt::now(),
        };
        let outcome = wt::call(transaction, &command).await?;
        match outcome.status.as_str() {
            "applied" => {
                record_playlist_adds(
                    transaction,
                    actor_sc_user_id,
                    idempotency_key,
                    &outcome.added,
                )
                .await?;
                Ok(JournalOutcome {
                    appended: usize::try_from(outcome.appended).map_err(|_| {
                        AppError::internal("playlist journal appended a negative count")
                    })?,
                    projection_revision: outcome.projection_revision,
                })
            }
            "replayed" => Ok(JournalOutcome {
                appended: 0,
                projection_revision: outcome.projection_revision,
            }),
            _ => Err(outcome
                .code
                .as_deref()
                .and_then(refused)
                .unwrap_or_else(|| AppError::internal("playlist journal refused the edit"))),
        }
    }
}

fn journal_entry(
    idempotency_key: Uuid,
    index: usize,
    operation: &Operation,
) -> wt::MembershipOperation {
    wt::MembershipOperation {
        operation_id: Uuid::now_v7(),
        idempotency_key: operation_key(idempotency_key, index),
        fingerprint: hex::encode(operation.fingerprint()),
        kind: operation.kind().to_owned(),
        track_id: operation.track_id().map(str::to_owned),
        left_anchor_track_id: operation.left_anchor().map(str::to_owned),
        right_anchor_track_id: operation.right_anchor().map(str::to_owned),
        boundary: operation.boundary().map(str::to_owned),
        ordered_track_ids: operation.ordered_track_ids().map(<[String]>::to_vec),
    }
}

fn refused(code: &str) -> Option<AppError> {
    match code {
        REVISION_CONFLICT => Some(revision_conflict()),
        UNKNOWN_TRACK => Some(unknown_track()),
        NOT_FOUND => Some(AppError::not_found("Playlist not found")),
        _ => None,
    }
}

fn revision_conflict() -> AppError {
    AppError::coded(
        StatusCode::CONFLICT,
        REVISION_CONFLICT,
        "playlist membership changed since the submitted revision",
    )
}

fn unknown_track() -> AppError {
    AppError::coded(
        StatusCode::NOT_FOUND,
        UNKNOWN_TRACK,
        "a submitted track is not in the catalog yet",
    )
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
        return Err(unknown_track());
    }
    Ok(())
}

fn operation_key(idempotency_key: Uuid, index: usize) -> Uuid {
    Uuid::new_v5(&idempotency_key, &(index as u64).to_be_bytes())
}
