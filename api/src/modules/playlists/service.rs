use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::cache::ListPageResult;
use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};
use crate::modules::auth::{TokenKind, TokenProvider, try_with_chain};
use crate::modules::cold_refresh::collection::CollectionPage;
use crate::modules::cold_refresh::{ColdRefreshService, PLAYLIST_REPOSTERS};
use crate::modules::playlists::edit::{MembershipRequest, TrackEdit};
use crate::modules::playlists::journal::PlaylistJournal;
use crate::modules::playlists::membership::PlaylistMembership;
use crate::modules::playlists::{PlaylistMembershipStatus, PlaylistRepository};
use crate::modules::sync_queue::SyncQueueService;
use crate::sc::ScClient;

const MAX_PLAYLIST_TRACKS: i64 = 20_000;

pub struct PlaylistsService {
    sc: ScClient,
    pg: PgPool,
    sync_queue: Arc<SyncQueueService>,
    cold_refresh: Arc<ColdRefreshService>,
    tokens: Arc<TokenProvider>,
    membership: PlaylistMembership,
    journal: PlaylistJournal,
    mutations: super::mutations::PlaylistMutations,
}

#[derive(Debug, Serialize)]
pub struct PlaylistTracksPage {
    #[serde(flatten)]
    pub page: ListPageResult<Value>,
    pub sync: PlaylistMembershipStatus,
}

pub struct PlaylistsDeps {
    pub sc: ScClient,
    pub pg: PgPool,
    pub sync_queue: Arc<SyncQueueService>,
    pub cold_refresh: Arc<ColdRefreshService>,
    pub tokens: Arc<TokenProvider>,
    pub background_jobs: Arc<crate::background_jobs::BackgroundJobs>,
}

impl PlaylistsService {
    pub fn new(deps: PlaylistsDeps) -> Arc<Self> {
        let membership = PlaylistMembership::new(deps.pg.clone(), deps.background_jobs);
        let journal = PlaylistJournal::new(deps.pg.clone());
        let mutations =
            super::mutations::PlaylistMutations::new(deps.pg.clone(), deps.sync_queue.clone());
        Arc::new(Self {
            sc: deps.sc,
            pg: deps.pg,
            sync_queue: deps.sync_queue,
            cold_refresh: deps.cold_refresh,
            tokens: deps.tokens,
            membership,
            journal,
            mutations,
        })
    }

    pub async fn create(&self, sc_user_id: &str, body: &Value) -> AppResult<Value> {
        let nonce = format!("new:{}", Uuid::new_v4());
        self.sync_queue
            .enqueue(sc_user_id, "playlist_create", &nonce, Some(body))
            .await?;
        Ok(json!({
            "status": "queued",
            "actionType": "playlist_create",
            "targetUrn": nonce,
        }))
    }

    pub async fn get_by_id(
        &self,
        session_id: Uuid,
        sc_user_id: &str,
        playlist_urn: &str,
        params: &[(String, String)],
    ) -> AppResult<Value> {
        let has_secret = params.iter().any(|(key, _)| key == "secret_token");
        self.get_by_id_with_fetch(sc_user_id, playlist_urn, has_secret, || async {
            let chain = self.tokens.chain(TokenKind::UserFirst(session_id)).await?;
            let mut metadata_params: Vec<_> = params
                .iter()
                .filter(|(key, _)| key != "show_tracks")
                .cloned()
                .collect();
            metadata_params.push(("show_tracks".into(), "false".into()));
            try_with_chain(&chain, |token| {
                let sc = self.sc.clone();
                let path = format!("/playlists/{playlist_urn}");
                let params = metadata_params.clone();
                async move { sc.api_get_value(&path, &token, Some(&params)).await }
            })
            .await
        })
        .await
    }

    pub(crate) async fn get_by_id_with_fetch<F, Fut>(
        &self,
        sc_user_id: &str,
        playlist_urn: &str,
        has_secret: bool,
        fetch: F,
    ) -> AppResult<Value>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<Value>>,
    {
        let canonical = super::mutations::target_urn(playlist_urn)?;
        let playlist_urn = canonical.as_str();
        self.mutations
            .ensure_read_access(sc_user_id, playlist_urn, has_secret)
            .await?;
        let repo = PlaylistRepository::new(self.pg.clone());
        let row = repo.find_by_urn(playlist_urn).await?;
        if row.as_ref().is_some_and(|row| row.deleted_at.is_some()) {
            return Err(AppError::not_found("Playlist not found"));
        }
        let known_locally = row.is_some();
        let local = row.filter(|row| {
            row.sharing == "public"
                || row
                    .owner_sc_user_id
                    .as_deref()
                    .is_some_and(|owner| extract_sc_id(owner) == extract_sc_id(sc_user_id))
        });
        let (current, verified_secret) = if let Some(row) = local {
            let _ = repo.touch_last_read(playlist_urn).await;
            if self.cold_refresh.is_playlist_stale(Some(row.sc_synced_at))
                && let Err(error) = crate::modules::cold_refresh::entity::enqueue_entity(
                    &self.pg,
                    backend_contracts::CatalogEntity::Playlist,
                    playlist_urn,
                    (row.sharing != "public").then_some(sc_user_id),
                )
                .await
            {
                tracing::debug!(%error, "catalog refresh enqueue deferred");
            }
            (row, false)
        } else if !has_secret {
            if known_locally {
                return Err(AppError::not_found("Playlist not found"));
            }
            return Err(crate::modules::cold_refresh::entity::refresh_pending(
                &self.pg,
                backend_contracts::CatalogEntity::Playlist,
                playlist_urn,
                None,
                "playlist_refresh_pending",
                "Playlist is being loaded",
            )
            .await);
        } else {
            let observation = catalog_ingest::Observation::begin(&self.pg).await?;
            let fetched = fetch().await?;
            crate::common::sc_payload::validate_entity_identity(
                &fetched,
                backend_contracts::CatalogEntity::Playlist,
                extract_sc_id(playlist_urn),
            )?;
            repo.upsert_from_sc(&fetched, observation).await?;
            let current = repo
                .find_by_urn(playlist_urn)
                .await?
                .ok_or_else(|| AppError::internal("Resolved playlist was not persisted"))?;
            (current, has_secret)
        };
        self.mutations
            .ensure_read_access(sc_user_id, playlist_urn, verified_secret)
            .await?;
        Ok(crate::modules::playlists::project_to_sc_shape(
            &current, None,
        ))
    }

    pub async fn edit_tracks(
        &self,
        sc_user_id: &str,
        playlist_urn: &str,
        request: MembershipRequest,
        idempotency_key: Uuid,
        page: i64,
        limit: i64,
    ) -> AppResult<PlaylistTracksPage> {
        let canonical = super::mutations::target_urn(playlist_urn)?;
        let playlist_urn = canonical.as_str();
        self.assert_owner(sc_user_id, playlist_urn).await?;
        self.journal_or_wake(playlist_urn, sc_user_id, request, idempotency_key)
            .await?;
        let (page, sync) = tokio::try_join!(
            self.project_page(playlist_urn, true, page, limit),
            self.membership.status(playlist_urn)
        )?;
        self.mutations
            .ensure_read_access(sc_user_id, playlist_urn, false)
            .await?;
        Ok(PlaylistTracksPage { page, sync })
    }

    pub async fn update(
        &self,
        sc_user_id: &str,
        playlist_urn: &str,
        body: &Value,
        replace: bool,
        idempotency_key: Uuid,
    ) -> AppResult<Value> {
        let canonical = super::mutations::target_urn(playlist_urn)?;
        let playlist_urn = canonical.as_str();
        let envelope = body
            .as_object()
            .ok_or_else(|| AppError::bad_request("expected a playlist object"))?;
        if envelope
            .keys()
            .any(|key| !matches!(key.as_str(), "playlist" | "expectedProjectionRevision"))
        {
            return Err(AppError::bad_request("unsupported playlist update field"));
        }
        let mut fields = envelope
            .get("playlist")
            .and_then(Value::as_object)
            .cloned()
            .filter(|fields| !fields.is_empty())
            .ok_or_else(|| AppError::bad_request("playlist update is empty"))?;
        let submitted = submitted_track_urns(body)?;
        fields.remove("tracks");
        let metadata = if fields.is_empty() {
            None
        } else {
            Some(
                catalog_ingest::PlaylistUpdate::parse(&json!({"playlist": fields}))
                    .map_err(AppError::bad_request)?,
            )
        };
        let membership = match submitted {
            Some(submitted) => {
                let track_ids = crate::modules::playlists::edit::track_ids_of(&submitted)?;
                let edit = if replace {
                    TrackEdit::Replace { track_ids }
                } else {
                    TrackEdit::Order { track_ids }
                };
                Some((
                    MembershipRequest {
                        edit,
                        expected_projection_revision: submitted_revision(body)?,
                    },
                    idempotency_key,
                ))
            }
            None => None,
        };
        let result = self
            .mutations
            .update(sc_user_id, playlist_urn, metadata.as_ref(), membership)
            .await;
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                if error.public_code() == crate::modules::playlists::journal::AWAITING_BASELINE {
                    self.membership
                        .enqueue_observation_if_due(playlist_urn)
                        .await;
                }
                return Err(error);
            }
        };
        let mut response = json!({
            "status": if outcome.metadata_queued { "queued" } else { "ok" },
            "targetUrn": outcome.target_urn,
        });
        if outcome.metadata_queued {
            response["actionType"] = json!("playlist_update");
        }
        if let Some(journal) = outcome.journal {
            response["appliedOperations"] = json!(journal.appended);
            response["projectionRevision"] = json!(journal.projection_revision);
            response["sync"] = json!(self.membership.status(&outcome.target_urn).await?);
        }
        Ok(response)
    }

    async fn journal_or_wake(
        &self,
        playlist_urn: &str,
        sc_user_id: &str,
        request: MembershipRequest,
        idempotency_key: Uuid,
    ) -> AppResult<crate::modules::playlists::journal::JournalOutcome> {
        match self
            .journal
            .append(
                playlist_urn,
                extract_sc_id(sc_user_id),
                request,
                idempotency_key,
            )
            .await
        {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                if error.public_code() == crate::modules::playlists::journal::AWAITING_BASELINE {
                    self.membership
                        .enqueue_observation_if_due(playlist_urn)
                        .await;
                }
                Err(error)
            }
        }
    }

    async fn assert_owner(&self, sc_user_id: &str, playlist_urn: &str) -> AppResult<()> {
        let me = extract_sc_id(sc_user_id);
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let owns = sqlx::query_file_scalar!(
            "queries/playlists/assert_owner.sql",
            me,
            playlist_urn,
            &variants
        )
        .fetch_one(&self.pg)
        .await?;
        if !owns {
            return Err(AppError::not_found("Playlist not found"));
        }
        Ok(())
    }

    async fn project_page(
        &self,
        playlist_urn: &str,
        can_see_private: bool,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let repo = PlaylistRepository::new(self.pg.clone());
        let offset = page
            .checked_mul(limit)
            .and_then(|offset| offset.checked_add(limit).map(|end| (offset, end)))
            .filter(|(_, end)| *end <= MAX_PLAYLIST_TRACKS)
            .map(|(offset, _)| offset)
            .ok_or_else(|| AppError::bad_request("playlist track page is out of range"))?;
        let ids = repo.page_track_ids(playlist_urn, offset, limit + 1).await?;
        let has_more = ids.len() as i64 > limit;
        let page_ids: Vec<String> = ids.into_iter().take(limit as usize).collect();
        let projected = if can_see_private {
            crate::modules::tracks::project_many(&self.pg, &page_ids).await?
        } else {
            crate::modules::tracks::project_many_public(&self.pg, &page_ids).await?
        };
        let collection: Vec<Value> = projected.into_iter().flatten().collect();
        Ok(ListPageResult {
            collection,
            page,
            page_size: limit,
            has_more,
        })
    }

    pub async fn set_sharing(
        &self,
        sc_user_id: &str,
        playlist_urn: &str,
        sharing: &str,
    ) -> AppResult<Value> {
        let metadata =
            catalog_ingest::PlaylistUpdate::parse(&json!({"playlist": {"sharing": sharing}}))
                .map_err(AppError::bad_request)?;
        let outcome = self
            .mutations
            .update(sc_user_id, playlist_urn, Some(&metadata), None)
            .await?;
        Ok(
            json!({"status": "queued", "actionType": "playlist_update", "targetUrn": outcome.target_urn}),
        )
    }

    pub async fn delete(&self, sc_user_id: &str, playlist_urn: &str) -> AppResult<Value> {
        self.mutations.delete(sc_user_id, playlist_urn).await
    }

    pub async fn get_tracks(
        &self,
        sc_user_id: &str,
        playlist_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<PlaylistTracksPage> {
        let canonical = super::mutations::target_urn(playlist_urn)?;
        let playlist_urn = canonical.as_str();
        let repo = crate::modules::playlists::PlaylistRepository::new(self.pg.clone());

        let viewer = crate::common::sc_ids::extract_sc_id(sc_user_id);
        let guard_private = |row: &crate::modules::playlists::PlaylistRow| -> AppResult<()> {
            if row.deleted_at.is_some()
                || (row.sharing != "public" && row.owner_sc_user_id.as_deref() != Some(viewer))
            {
                return Err(AppError::not_found("Playlist not found"));
            }
            Ok(())
        };

        let playlist_row = repo
            .find_by_urn(playlist_urn)
            .await?
            .ok_or_else(|| AppError::not_found("Playlist not found"))?;
        guard_private(&playlist_row)?;
        let can_see_private = playlist_row.sharing != "public"
            || playlist_row.owner_sc_user_id.as_deref() == Some(viewer);

        let (page, sync) = tokio::try_join!(
            self.project_page(playlist_urn, can_see_private, page, limit),
            self.membership.status(playlist_urn)
        )?;
        self.membership
            .enqueue_observation_if_due(playlist_urn)
            .await;
        self.mutations
            .ensure_read_access(sc_user_id, playlist_urn, false)
            .await?;
        Ok(PlaylistTracksPage { page, sync })
    }

    pub async fn get_reposters(
        &self,
        sc_user_id: &str,
        playlist_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let canonical = super::mutations::target_urn(playlist_urn)?;
        let playlist_urn = canonical.as_str();
        self.mutations
            .ensure_read_access(sc_user_id, playlist_urn, false)
            .await?;
        self.cold_refresh
            .audience_page(PLAYLIST_REPOSTERS, playlist_urn, page, limit)
            .await
    }
}

fn submitted_track_urns(body: &Value) -> AppResult<Option<Vec<String>>> {
    let Some(tracks) = body
        .get("playlist")
        .and_then(|playlist| playlist.get("tracks"))
    else {
        return Ok(None);
    };
    let Some(tracks) = tracks.as_array() else {
        return Err(AppError::bad_request("playlist tracks must be an array"));
    };
    let mut submitted = Vec::with_capacity(tracks.len());
    for track in tracks {
        let value = match track {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            Value::Object(_) => track
                .get("urn")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| track.get("id").and_then(scalar_id)),
            _ => None,
        };
        submitted.push(
            value.ok_or_else(|| AppError::bad_request("playlist track entry has no identifier"))?,
        );
    }
    Ok(Some(submitted))
}

fn submitted_revision(body: &Value) -> AppResult<Option<i64>> {
    let Some(value) = body.get("expectedProjectionRevision") else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| AppError::bad_request("expectedProjectionRevision must be an integer")),
        Value::String(text) => text
            .parse::<i64>()
            .map(Some)
            .map_err(|_| AppError::bad_request("expectedProjectionRevision must be an integer")),
        _ => Err(AppError::bad_request(
            "expectedProjectionRevision must be an integer",
        )),
    }
}

fn scalar_id(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
