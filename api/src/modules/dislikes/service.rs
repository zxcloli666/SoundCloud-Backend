use std::sync::Arc;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::common::sc_ids::normalize_sc_track_id;
use crate::error::{AppError, AppResult};
use crate::modules::events::EventsService;
use crate::modules::resolve::CatalogMiss;

pub struct DislikesService {
    pg: PgPool,
    events: Arc<EventsService>,
    miss: Arc<CatalogMiss>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DislikesPage {
    pub collection: Vec<Value>,
    pub next_href: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusResult {
    pub status: String,
}

impl DislikesService {
    pub fn new(pg: PgPool, events: Arc<EventsService>, miss: Arc<CatalogMiss>) -> Arc<Self> {
        Arc::new(Self { pg, events, miss })
    }

    pub async fn add(
        &self,
        session_id: uuid::Uuid,
        sc_user_id: &str,
        sc_track_id: &str,
        track_data: Option<&Value>,
    ) -> AppResult<StatusResult> {
        let Some(id) = normalize_sc_track_id(sc_track_id) else {
            return Ok(StatusResult {
                status: "invalid".into(),
            });
        };
        self.miss
            .track_soon(
                session_id,
                &id,
                crate::modules::tracks::TrackPriority::Discovery,
            )
            .await?;

        let inserted: Option<(uuid::Uuid,)> = sqlx::query_as(
            "INSERT INTO disliked_tracks (sc_user_id, sc_track_id, track_data) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (sc_user_id, sc_track_id) DO NOTHING \
             RETURNING id",
        )
        .bind(sc_user_id)
        .bind(&id)
        .bind(track_data)
        .fetch_optional(&self.pg)
        .await?;

        if inserted.is_some() {
            self.events.record(sc_user_id, &id, "dislike", None).await?;
        }
        Ok(StatusResult {
            status: "ok".into(),
        })
    }

    pub async fn remove(&self, sc_user_id: &str, sc_track_id: &str) -> AppResult<StatusResult> {
        let Some(id) = normalize_sc_track_id(sc_track_id) else {
            return Ok(StatusResult {
                status: "invalid".into(),
            });
        };
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        sqlx::query_file!("queries/dislikes/service/remove.sql", &variants, &id)
            .execute(&self.pg)
            .await?;
        Ok(StatusResult {
            status: "removed".into(),
        })
    }

    pub async fn is_disliked(&self, sc_user_id: &str, sc_track_id: &str) -> AppResult<bool> {
        let Some(id) = normalize_sc_track_id(sc_track_id) else {
            return Ok(false);
        };
        self.is_disliked_by_user_id(sc_user_id, &id).await
    }

    pub async fn is_disliked_by_user_id(
        &self,
        sc_user_id: &str,
        sc_track_id: &str,
    ) -> AppResult<bool> {
        let Some(id) = normalize_sc_track_id(sc_track_id) else {
            return Ok(false);
        };
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let row = sqlx::query_file_scalar!(
            "queries/dislikes/service/is_disliked_by_user_id.sql",
            &variants,
            &id
        )
        .fetch_optional(&self.pg)
        .await?;
        Ok(row.is_some())
    }

    pub async fn list_ids_by_user_id(
        &self,
        sc_user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<String>> {
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let rows = sqlx::query_file_scalar!(
            "queries/dislikes/service/list_ids_by_user_id.sql",
            &variants,
            limit
        )
        .fetch_all(&self.pg)
        .await?;
        Ok(rows)
    }

    pub async fn find_all(
        &self,
        sc_user_id: &str,
        limit: i64,
        cursor: Option<&str>,
    ) -> AppResult<DislikesPage> {
        let cursor_dt = match cursor {
            Some(s) => Some(parse_cursor(s)?),
            None => None,
        };

        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let rows: Vec<(String, Option<Value>, NaiveDateTime)> = if let Some(dt) = cursor_dt {
            sqlx::query_file!(
                "queries/dislikes/service/find_all_after_cursor.sql",
                &variants,
                dt,
                limit + 1
            )
            .fetch_all(&self.pg)
            .await?
            .into_iter()
            .map(|r| (r.sc_track_id, r.track_data, r.created_at))
            .collect()
        } else {
            sqlx::query_file!(
                "queries/dislikes/service/find_all.sql",
                &variants,
                limit + 1
            )
            .fetch_all(&self.pg)
            .await?
            .into_iter()
            .map(|r| (r.sc_track_id, r.track_data, r.created_at))
            .collect()
        };

        let has_more = rows.len() as i64 > limit;
        let slice: Vec<(String, Option<Value>, NaiveDateTime)> =
            rows.into_iter().take(limit as usize).collect();
        let next_href = if has_more {
            slice.last().map(|(_, _, dt)| {
                let iso = DateTime::<Utc>::from_naive_utc_and_offset(*dt, Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                format!("?limit={limit}&cursor={iso}")
            })
        } else {
            None
        };
        let collection = self.project(sc_user_id, slice).await?;
        Ok(DislikesPage {
            collection,
            next_href,
        })
    }
}

impl DislikesService {
    async fn project(
        &self,
        sc_user_id: &str,
        rows: Vec<(String, Option<Value>, NaiveDateTime)>,
    ) -> AppResult<Vec<Value>> {
        let ids: Vec<String> = rows.iter().map(|(id, _, _)| id.clone()).collect();
        let mut projected = crate::modules::tracks::project_many_public(&self.pg, &ids).await?;
        let mut found: Vec<Value> = projected.iter_mut().filter_map(Option::take).collect();
        crate::modules::enrich::dto::apply_to_tracks(&self.pg, &mut found).await?;
        crate::modules::likes::cold::apply_user_favorite_flag(&self.pg, sc_user_id, &mut found)
            .await?;
        let mut found: std::collections::HashMap<String, Value> = found
            .into_iter()
            .filter_map(|track| {
                let id = normalize_sc_track_id(track.get("urn")?.as_str()?)?;
                Some((id, track))
            })
            .collect();
        Ok(rows
            .into_iter()
            .filter_map(|(id, stored, _)| {
                found.remove(&id).or_else(|| stored_fallback(&id, stored))
            })
            .collect())
    }
}

fn stored_fallback(sc_track_id: &str, stored: Option<Value>) -> Option<Value> {
    let entity = crate::common::sc_ids::EntityRef::track(sc_track_id)?;
    let mut stored = stored.filter(|stored| {
        stored
            .get("urn")
            .and_then(Value::as_str)
            .and_then(crate::common::sc_ids::EntityRef::parse_urn)
            == Some(entity)
    })?;
    let object = stored.as_object_mut()?;
    object.insert("id".into(), serde_json::json!(entity.id()));
    object.insert("urn".into(), Value::String(entity.urn()));
    object.remove("_scd_meta");
    Some(stored)
}

fn parse_cursor(s: &str) -> AppResult<NaiveDateTime> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.naive_utc())
        .map_err(|e| AppError::bad_request(format!("invalid cursor: {e}")))
}
