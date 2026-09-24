use std::sync::Arc;

use backend_contracts::CatalogEntity;
use chrono::NaiveDateTime;
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::likes::cold as likes_cold;
use crate::modules::playlists::{PlaylistRepository, project_to_sc_shape as project_playlist};
use crate::modules::tracks::repository::project_many_public;
use crate::modules::users::{UserRepository, project_to_sc_shape as project_user};

const MAX_PLAYLIST_TRACKS: i64 = 20_000;

async fn enqueue_featured(
    connection: &mut sqlx::PgConnection,
    type_: &str,
    urn: &str,
) -> AppResult<()> {
    let entity = FeaturedItemType::parse(type_)
        .ok_or_else(|| AppError::bad_request("Unknown featured entity type"))?
        .catalog_entity();
    if urn != entity.urn(crate::common::sc_ids::extract_sc_id(urn)) {
        return Err(AppError::bad_request(
            "scUrn must be a canonical SoundCloud URN matching type",
        ));
    }
    crate::modules::cold_refresh::entity::enqueue_entity_in(connection, entity, urn, None).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeaturedItemType {
    Track,
    Playlist,
    User,
}

impl FeaturedItemType {
    fn catalog_entity(self) -> CatalogEntity {
        match self {
            Self::Track => CatalogEntity::Track,
            Self::Playlist => CatalogEntity::Playlist,
            Self::User => CatalogEntity::User,
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "track" => Some(Self::Track),
            "playlist" => Some(Self::Playlist),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct FeaturedItem {
    pub id: Uuid,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub type_: String,
    #[serde(rename = "scUrn")]
    pub sc_urn: String,
    pub weight: i32,
    pub active: bool,
    #[serde(rename = "createdAt")]
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeaturedResult {
    #[serde(rename = "type")]
    pub type_: String,
    pub data: Value,
}

pub struct FeaturedService {
    pg: sqlx::PgPool,
}

impl FeaturedService {
    pub fn new(pg: sqlx::PgPool) -> Arc<Self> {
        Arc::new(Self { pg })
    }

    pub async fn find_all(&self) -> AppResult<Vec<FeaturedItem>> {
        let rows = sqlx::query_file!("queries/featured/service/find_all.sql")
            .fetch_all(&self.pg)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| FeaturedItem {
                id: r.id,
                type_: r.item_type,
                sc_urn: r.sc_urn,
                weight: r.weight,
                active: r.active,
                created_at: r.created_at,
            })
            .collect())
    }

    pub async fn create(
        &self,
        type_: &str,
        sc_urn: &str,
        weight: Option<i32>,
        active: Option<bool>,
    ) -> AppResult<FeaturedItem> {
        if FeaturedItemType::parse(type_).is_none() {
            return Err(AppError::bad_request(
                "type must be one of: track, playlist, user",
            ));
        }
        let mut transaction = self.pg.begin().await?;
        let row = sqlx::query_file!(
            "queries/featured/service/create.sql",
            type_,
            sc_urn,
            weight.unwrap_or(1),
            active.unwrap_or(true)
        )
        .fetch_one(&mut *transaction)
        .await?;
        enqueue_featured(&mut transaction, &row.item_type, &row.sc_urn).await?;
        transaction.commit().await?;
        Ok(FeaturedItem {
            id: row.id,
            type_: row.item_type,
            sc_urn: row.sc_urn,
            weight: row.weight,
            active: row.active,
            created_at: row.created_at,
        })
    }

    pub async fn update(
        &self,
        id: &str,
        type_: Option<&str>,
        sc_urn: Option<&str>,
        weight: Option<i32>,
        active: Option<bool>,
    ) -> AppResult<FeaturedItem> {
        if let Some(t) = type_
            && FeaturedItemType::parse(t).is_none()
        {
            return Err(AppError::bad_request(
                "type must be one of: track, playlist, user",
            ));
        }
        let uuid = Uuid::parse_str(id)
            .map_err(|_| AppError::not_found(format!("featured item {id} not found")))?;
        let mut transaction = self.pg.begin().await?;
        let row = sqlx::query_file!(
            "queries/featured/service/update.sql",
            uuid,
            type_,
            sc_urn,
            weight,
            active
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let row =
            row.ok_or_else(|| AppError::not_found(format!("featured item {id} not found")))?;
        enqueue_featured(&mut transaction, &row.item_type, &row.sc_urn).await?;
        transaction.commit().await?;
        Ok(FeaturedItem {
            id: row.id,
            type_: row.item_type,
            sc_urn: row.sc_urn,
            weight: row.weight,
            active: row.active,
            created_at: row.created_at,
        })
    }

    pub async fn remove(&self, id: &str) -> AppResult<()> {
        let uuid = match Uuid::parse_str(id) {
            Ok(u) => u,
            Err(_) => return Ok(()),
        };
        sqlx::query_file!("queries/featured/service/remove.sql", uuid)
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    pub async fn pick(&self, sc_user_id: &str) -> AppResult<Option<FeaturedResult>> {
        let Some(item) = sqlx::query_file!("queries/featured/service/pick_active.sql")
            .fetch_optional(&self.pg)
            .await?
        else {
            return Ok(None);
        };
        let Some(data) = self
            .resolve(&item.item_type, &item.sc_urn, sc_user_id)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(FeaturedResult {
            type_: item.item_type,
            data,
        }))
    }

    async fn resolve(
        &self,
        item_type: &str,
        urn: &str,
        sc_user_id: &str,
    ) -> AppResult<Option<Value>> {
        let id = crate::common::sc_ids::extract_sc_id(urn);
        match item_type {
            "track" => {
                let mut single: Vec<Value> = project_many_public(&self.pg, &[id.to_owned()])
                    .await?
                    .into_iter()
                    .flatten()
                    .collect();
                likes_cold::apply_user_favorite_flag(&self.pg, sc_user_id, &mut single).await?;
                Ok(single.into_iter().next())
            }
            "playlist" => {
                let repository = PlaylistRepository::new(self.pg.clone());
                let Some(row) = repository
                    .find_by_urn(urn)
                    .await?
                    .filter(|row| row.sharing == "public")
                else {
                    return Ok(None);
                };
                let owner = match row.owner_urn.as_deref() {
                    Some(urn) => UserRepository::new(self.pg.clone())
                        .find_by_urn(urn)
                        .await?
                        .map(|row| project_user(&row)),
                    None => None,
                };
                let ids = repository
                    .page_track_ids(urn, 0, MAX_PLAYLIST_TRACKS)
                    .await?;
                let mut tracks: Vec<Value> = project_many_public(&self.pg, &ids)
                    .await?
                    .into_iter()
                    .flatten()
                    .collect();
                likes_cold::apply_user_favorite_flag(&self.pg, sc_user_id, &mut tracks).await?;
                let mut playlist = project_playlist(&row, owner.as_ref());
                if let Some(obj) = playlist.as_object_mut() {
                    obj.insert("tracks".into(), Value::Array(tracks));
                }
                Ok(Some(playlist))
            }
            "user" => Ok(UserRepository::new(self.pg.clone())
                .find_by_urn(urn)
                .await?
                .map(|row| project_user(&row))),
            other => Err(AppError::internal(format!(
                "unknown featured type: {other}"
            ))),
        }
    }
}
