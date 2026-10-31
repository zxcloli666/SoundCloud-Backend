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
        crate::db::retry_conflicts(|| self.update_once(user, target, metadata, membership.clone()))
            .await
    }

    async fn update_once(
        &self,
        user: &str,
        target: &str,
        metadata: Option<&PlaylistUpdate>,
        membership: Option<(MembershipRequest, Uuid)>,
    ) -> AppResult<PlaylistMutationOutcome> {
        let target = target_urn(target)?;
        let observation = catalog_ingest::Observation::begin(&self.pg).await?;
        let mut tx = self.begin(&target).await?;
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
                    return Err(error);
                }
            },
            None => None,
        };
        if let Some(metadata) = metadata {
            let edit = wt::PlaylistEdit {
                urn: target.clone(),
                owner: extract_sc_id(user).to_owned(),
                desired: metadata.desired().clone(),
                observation: observation.sequence(),
                at: wt::now(),
            };
            if wt::call(&mut tx, &edit).await?.is_none() {
                return Err(AppError::not_found("Playlist not found"));
            }
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
        tx.commit().await?;
        Ok(PlaylistMutationOutcome {
            target_urn: target,
            metadata_queued: metadata.is_some(),
            journal,
        })
    }

    pub(super) async fn delete(&self, user: &str, target: &str) -> AppResult<Value> {
        crate::db::retry_conflicts(|| self.delete_once(user, target)).await
    }

    async fn delete_once(&self, user: &str, target: &str) -> AppResult<Value> {
        let target = target_urn(target)?;
        let observation = catalog_ingest::Observation::begin(&self.pg).await?;
        let mut tx = self.begin(&target).await?;
        let delete = wt::PlaylistDelete {
            urn: target.clone(),
            owner: extract_sc_id(user).to_owned(),
            observation: observation.sequence(),
            at: wt::now(),
        };
        if wt::call(&mut tx, &delete).await?.is_none() {
            return Err(AppError::not_found("Playlist not found"));
        }
        let owned = wt::OwnedDelete::Playlist {
            variants: crate::common::sc_ids::user_id_variants(user),
            playlist_urn: target.clone(),
        };
        wt::call(&mut tx, &owned).await?;
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

pub(crate) fn target_urn(target: &str) -> AppResult<String> {
    crate::common::sc_ids::require_ref(crate::common::sc_ids::EntityKind::Playlist, target)
        .map(crate::common::sc_ids::EntityRef::urn)
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
