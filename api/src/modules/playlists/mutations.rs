use std::sync::Arc;

use catalog_ingest::PlaylistUpdate;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};
use crate::modules::sync_queue::SyncQueueService;

use super::edit::MembershipRequest;
use super::journal::{JournalOutcome, PlaylistJournal};

pub(super) struct PlaylistMutations {
    pg: PgPool,
    queue: Arc<SyncQueueService>,
    journal: PlaylistJournal,
}

pub(super) struct PlaylistMutationOutcome {
    pub target_urn: String,
    pub metadata_queued: bool,
    pub journal: Option<JournalOutcome>,
}

impl PlaylistMutations {
    pub(super) fn new(pg: PgPool, queue: Arc<SyncQueueService>) -> Self {
        Self {
            journal: PlaylistJournal::new(pg.clone()),
            pg,
            queue,
        }
    }

    async fn begin(&self, target: &str) -> AppResult<Transaction<'_, Postgres>> {
        let mut tx = self.pg.begin().await?;
        sqlx::query_file_scalar!("queries/playlists/service/lock_mutation.sql", target)
            .fetch_one(&mut *tx)
            .await?;
        Ok(tx)
    }

    pub(super) async fn update(
        &self,
        user: &str,
        target: &str,
        metadata: Option<&PlaylistUpdate>,
        membership: Option<(MembershipRequest, Uuid)>,
    ) -> AppResult<PlaylistMutationOutcome> {
        let target = target_urn(target)?;
        let mut tx = self.begin(&target).await?;
        if let Some(metadata) = metadata {
            self.queue
                .enqueue_on(
                    &mut tx,
                    user,
                    "playlist_update",
                    &target,
                    Some(metadata.body()),
                )
                .await?;
            let pending = sqlx::query_file_scalar!(
                "queries/playlists/service/pending_update.sql",
                extract_sc_id(user),
                &target
            )
            .fetch_one(&mut *tx)
            .await?;
            PlaylistUpdate::parse(&pending).map_err(AppError::bad_request)?;
        }
        sqlx::query_file_scalar!("queries/playlists/service/lock_membership.sql", &target)
            .fetch_optional(&mut *tx)
            .await?;
        let owns = sqlx::query_file_scalar!(
            "queries/playlists/assert_owner.sql",
            extract_sc_id(user),
            &target,
            &crate::common::sc_ids::user_id_variants(user)
        )
        .fetch_one(&mut *tx)
        .await?;
        if !owns {
            return Err(AppError::not_found("Playlist not found"));
        }
        let journal = match membership {
            Some((request, key)) => match self
                .journal
                .append_in(&mut tx, &target, extract_sc_id(user), request, key)
                .await
            {
                Ok(outcome) => Some(outcome),
                Err(error) => {
                    tx.rollback().await?;
                    if error.public_code() == super::journal::AWAITING_BASELINE {
                        self.request_baseline(user, &target).await?;
                    }
                    return Err(error);
                }
            },
            None => None,
        };
        if let Some(metadata) = metadata {
            let changed = sqlx::query_file_scalar!(
                "queries/playlists/service/apply_update.sql",
                &target,
                extract_sc_id(user),
                metadata.desired()
            )
            .fetch_optional(&mut *tx)
            .await?;
            if changed.is_none() {
                return Err(AppError::not_found("Playlist not found"));
            }
        }
        tx.commit().await?;
        Ok(PlaylistMutationOutcome {
            target_urn: target,
            metadata_queued: metadata.is_some(),
            journal,
        })
    }

    async fn request_baseline(&self, user: &str, target: &str) -> AppResult<()> {
        let mut tx = self.begin(target).await?;
        sqlx::query_file_scalar!("queries/playlists/service/lock_membership.sql", target)
            .fetch_optional(&mut *tx)
            .await?;
        let owns = sqlx::query_file_scalar!(
            "queries/playlists/assert_owner.sql",
            extract_sc_id(user),
            target,
            &crate::common::sc_ids::user_id_variants(user)
        )
        .fetch_one(&mut *tx)
        .await?;
        if owns {
            sqlx::query_file!("queries/playlists/mark_reconcile_due.sql", target)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn delete(&self, user: &str, target: &str) -> AppResult<Value> {
        let target = target_urn(target)?;
        let mut tx = self.begin(&target).await?;
        sqlx::query_file!(
            "queries/playlists/service/cancel_updates.sql",
            extract_sc_id(user),
            &target
        )
        .execute(&mut *tx)
        .await?;
        self.queue
            .enqueue_on(&mut tx, user, "playlist_delete", &target, None)
            .await?;
        sqlx::query_file_scalar!("queries/playlists/service/lock_membership.sql", &target)
            .fetch_optional(&mut *tx)
            .await?;
        let changed = sqlx::query_file_scalar!(
            "queries/playlists/service/apply_delete.sql",
            &target,
            extract_sc_id(user)
        )
        .fetch_optional(&mut *tx)
        .await?;
        if changed.is_none() {
            return Err(AppError::not_found("Playlist not found"));
        }
        sqlx::query_file!("queries/playlists/service/retire_membership.sql", &target)
            .execute(&mut *tx)
            .await?;
        sqlx::query_file!(
            "queries/playlists/service/delete_owned.sql",
            &crate::common::sc_ids::user_id_variants(user),
            &target
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(json!({"status": "queued", "actionType": "playlist_delete", "targetUrn": target}))
    }

    pub(super) async fn ensure_read_access(
        &self,
        user: &str,
        target: &str,
        secret: bool,
    ) -> AppResult<()> {
        let target = target_urn(target)?;
        let access = sqlx::query_file!(
            "queries/playlists/service/read_access.sql",
            &target,
            extract_sc_id(user)
        )
        .fetch_optional(&self.pg)
        .await?;
        if let Some(access) = access
            && (access.deleted
                || (access.can_read != Some(true) && !(secret && access.secret_ready)))
        {
            return Err(AppError::not_found("Playlist not found"));
        }
        Ok(())
    }
}

pub(super) fn target_urn(target: &str) -> AppResult<String> {
    let id = extract_sc_id(target);
    let payload = backend_contracts::CatalogRefreshPayload {
        entity: backend_contracts::CatalogEntity::Playlist,
        sc_id: id.to_owned(),
        owner_id: None,
    };
    let canonical = payload.entity.urn(id);
    if !payload.is_valid() || (target != id && target != canonical) {
        return Err(AppError::bad_request("invalid playlist identifier"));
    }
    Ok(canonical)
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
