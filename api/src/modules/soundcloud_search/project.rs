use std::collections::{HashMap, HashSet};

use sc_transport::SearchType;
use serde_json::Value;
use sqlx::PgPool;

use crate::cache::ListPageResult;
use crate::common::sc_ids::{EntityKind, EntityRef};
use crate::error::AppResult;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::playlists::PlaylistRow;
use crate::modules::search::catalog::project_playlists_with_owners;
use crate::modules::users::{UserRow, project_to_sc_shape as project_user};

pub(crate) async fn project_page(
    pg: &PgPool,
    ty: SearchType,
    page: ListPageResult<Value>,
) -> AppResult<ListPageResult<Value>> {
    let kind = kind_of(ty);
    let refs = refs_of(kind, &page.collection);
    ingest(pg, kind, &page.collection).await;
    let collection = match kind {
        EntityKind::Track => tracks(pg, &refs).await?,
        EntityKind::Playlist => playlists(pg, &refs).await?,
        EntityKind::User => users(pg, &refs).await?,
    };
    Ok(ListPageResult { collection, ..page })
}

fn kind_of(ty: SearchType) -> EntityKind {
    match ty {
        SearchType::Tracks => EntityKind::Track,
        SearchType::Users => EntityKind::User,
        _ => EntityKind::Playlist,
    }
}

pub(super) fn refs_of(kind: EntityKind, items: &[Value]) -> Vec<EntityRef> {
    let mut seen = HashSet::new();
    items
        .iter()
        .filter_map(|item| item_ref(kind, item))
        .filter(|entity| seen.insert(*entity))
        .collect()
}

fn item_ref(kind: EntityKind, item: &Value) -> Option<EntityRef> {
    if let Some(urn) = item.get("urn").and_then(Value::as_str) {
        return EntityRef::parse_urn(urn).filter(|entity| entity.kind() == kind);
    }
    match item.get("id")? {
        Value::Number(id) => EntityRef::new(kind, id.as_u64()?),
        Value::String(id) => EntityRef::parse(kind, id),
        _ => None,
    }
}

async fn ingest(pg: &PgPool, kind: EntityKind, items: &[Value]) {
    let observation = catalog_ingest::Observation::UNVERIFIED;
    let stored = match kind {
        EntityKind::Track => catalog_ingest::insert_absent_tracks(pg, items, observation).await,
        EntityKind::Playlist => {
            catalog_ingest::insert_absent_playlists(pg, items, observation).await
        }
        EntityKind::User => catalog_ingest::insert_absent_users(pg, items, observation).await,
    };
    match stored {
        Ok(stored) => tracing::debug!(
            tracks = stored.tracks.len(),
            playlists = stored.playlists.len(),
            users = stored.users.len(),
            "stored unseen SoundCloud search hits"
        ),
        Err(error) => tracing::warn!(%error, "SoundCloud search hits were not stored"),
    }
}

async fn tracks(pg: &PgPool, refs: &[EntityRef]) -> AppResult<Vec<Value>> {
    let ids: Vec<String> = refs.iter().map(|entity| entity.sc_id()).collect();
    let mut collection: Vec<Value> = crate::modules::tracks::project_many_public(pg, &ids)
        .await?
        .into_iter()
        .flatten()
        .collect();
    enrich_dto::apply_to_tracks(pg, &mut collection).await?;
    Ok(collection)
}

async fn playlists(pg: &PgPool, refs: &[EntityRef]) -> AppResult<Vec<Value>> {
    let urns: Vec<String> = refs.iter().map(|entity| entity.urn()).collect();
    let mut rows: HashMap<String, PlaylistRow> = sqlx::query_file_as!(
        PlaylistRow,
        "queries/playlists/project_many_public.sql",
        &urns
    )
    .fetch_all(pg)
    .await?
    .into_iter()
    .map(|row| (row.urn.clone(), row))
    .collect();
    let ordered: Vec<PlaylistRow> = urns.iter().filter_map(|urn| rows.remove(urn)).collect();
    project_playlists_with_owners(pg, ordered).await
}

async fn users(pg: &PgPool, refs: &[EntityRef]) -> AppResult<Vec<Value>> {
    let ids: Vec<String> = refs.iter().map(|entity| entity.sc_id()).collect();
    let rows: HashMap<String, UserRow> =
        sqlx::query_file_as!(UserRow, "queries/search/users_by_sc_ids.sql", &ids)
            .fetch_all(pg)
            .await?
            .into_iter()
            .map(|row| (row.sc_user_id.clone(), row))
            .collect();
    Ok(ids
        .iter()
        .filter_map(|id| rows.get(id))
        .map(project_user)
        .collect())
}
