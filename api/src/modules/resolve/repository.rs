use backend_contracts::CatalogEntity;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use super::input::{EntityKey, ResolveInput};
use crate::error::{AppError, AppResult};
use crate::modules::{playlists, tracks, users};

pub(super) struct LocalEntity {
    pub value: Value,
    pub synced_at: DateTime<Utc>,
    pub public: bool,
    pub owner: Option<String>,
}

pub(super) async fn find(pg: &PgPool, input: &ResolveInput) -> AppResult<Option<EntityKey>> {
    if input.entity.is_none() && input.permalinks.is_empty() {
        return Ok(None);
    }
    let urn = input.entity.as_ref().map(EntityKey::urn);
    let rows = sqlx::query_file_scalar!(
        "queries/resolve/find_entity.sql",
        urn.as_deref(),
        &input.permalinks
    )
    .fetch_all(pg)
    .await?;
    Ok(match rows.as_slice() {
        [urn] => EntityKey::parse(urn),
        _ => None,
    })
}

fn is_owner(owner: Option<&str>, viewer: Option<&str>) -> bool {
    owner.zip(viewer).is_some_and(|(owner, viewer)| {
        crate::common::sc_ids::extract_sc_id(owner) == crate::common::sc_ids::extract_sc_id(viewer)
    })
}

async fn user_projection(pg: &PgPool, id: Option<&str>) -> AppResult<Option<Value>> {
    let Some(id) = id else {
        return Ok(None);
    };
    Ok(users::UserRepository::new(pg.clone())
        .find_by_urn(id)
        .await?
        .as_ref()
        .map(users::project_to_sc_shape))
}

pub(super) async fn load(
    pg: &PgPool,
    key: &EntityKey,
    viewer: Option<&str>,
    verified_secret: bool,
) -> AppResult<LocalEntity> {
    let missing = || AppError::not_found("Resolved entity not found");
    match key.entity {
        CatalogEntity::Track => {
            let row =
                sqlx::query_file_as!(tracks::TrackRow, "queries/resolve/find_track.sql", &key.id)
                    .fetch_optional(pg)
                    .await?
                    .ok_or_else(missing)?;
            if row.deleted_at.is_some() {
                return Err(missing());
            }
            let public = row.sharing == "public";
            if !public && !is_owner(row.uploader_sc_user_id.as_deref(), viewer) {
                let access = sqlx::query_file!(
                    "queries/tracks/service/read_access.sql",
                    &key.id,
                    viewer.unwrap_or_default()
                )
                .fetch_optional(pg)
                .await?;
                if !verified_secret
                    || !access.is_some_and(|access| !access.deleted && access.secret_ready)
                {
                    return Err(missing());
                }
            }
            let uploader = user_projection(pg, row.uploader_sc_user_id.as_deref()).await?;
            Ok(LocalEntity {
                value: tracks::project_to_sc_shape(&row, uploader.as_ref()),
                synced_at: row.sc_synced_at,
                public,
                owner: row.uploader_sc_user_id,
            })
        }
        CatalogEntity::Playlist => {
            let row = playlists::PlaylistRepository::new(pg.clone())
                .find_by_urn(&key.urn())
                .await?
                .ok_or_else(missing)?;
            if row.deleted_at.is_some() {
                return Err(missing());
            }
            let public = row.sharing == "public";
            if !public && !is_owner(row.owner_sc_user_id.as_deref(), viewer) {
                let access = sqlx::query_file!(
                    "queries/playlists/service/read_access.sql",
                    &key.urn(),
                    viewer.unwrap_or_default()
                )
                .fetch_optional(pg)
                .await?;
                if !verified_secret
                    || !access.is_some_and(|access| !access.deleted && access.secret_ready)
                {
                    return Err(missing());
                }
            }
            let owner = user_projection(pg, row.owner_sc_user_id.as_deref()).await?;
            Ok(LocalEntity {
                value: playlists::project_to_sc_shape(&row, owner.as_ref()),
                synced_at: row.sc_synced_at,
                public,
                owner: row.owner_sc_user_id,
            })
        }
        CatalogEntity::User => {
            let row = users::UserRepository::new(pg.clone())
                .find_by_urn(&key.urn())
                .await?
                .ok_or_else(missing)?;
            Ok(LocalEntity {
                value: users::project_to_sc_shape(&row),
                synced_at: row.sc_synced_at,
                public: true,
                owner: None,
            })
        }
        CatalogEntity::Profile | CatalogEntity::WebProfiles => Err(missing()),
    }
}
