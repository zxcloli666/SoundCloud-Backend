use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value, json};
use sqlx::FromRow;
use sqlx::PgPool;

use crate::common::sc_payload::parse_id_or_string;
use crate::error::AppResult;

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)]
pub struct PlaylistRow {
    pub urn: String,
    pub sc_playlist_id: String,
    pub title: String,
    pub title_normalized: String,
    pub description: Option<String>,
    pub genre: Option<String>,
    pub tags: Vec<String>,
    pub artwork_url: Option<String>,
    pub permalink_url: Option<String>,
    pub owner_sc_user_id: Option<String>,
    pub owner_urn: Option<String>,
    pub owner_username: Option<String>,
    pub track_count: i32,
    pub duration_ms: Option<i64>,
    pub playlist_type: Option<String>,
    pub kind: Option<String>,
    pub sharing: String,
    pub sc_metadata: Value,
    pub deleted_at: Option<DateTime<Utc>>,
    pub release_year: Option<i16>,
    pub release_date: Option<NaiveDate>,
    pub label_name: Option<String>,
    pub likes_count_sc: Option<i64>,
    pub reposts_count_sc: Option<i64>,
    pub sc_created_at: Option<DateTime<Utc>>,
    pub sc_last_modified: Option<DateTime<Utc>>,
    pub sc_synced_at: DateTime<Utc>,
    pub last_read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct PlaylistRepository {
    pg: PgPool,
}

impl PlaylistRepository {
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    pub async fn find_by_urn(&self, urn: &str) -> AppResult<Option<PlaylistRow>> {
        let row = sqlx::query_file_as!(PlaylistRow, "queries/playlists/find_by_urn.sql", urn)
            .fetch_optional(&self.pg)
            .await?;
        Ok(row)
    }

    pub async fn touch_last_read(&self, urn: &str) -> AppResult<()> {
        sqlx::query_file!("queries/playlists/touch_last_read.sql", urn)
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    pub async fn upsert_from_sc(
        &self,
        payload: &Value,
        observation: catalog_ingest::Observation,
    ) -> AppResult<bool> {
        Ok(catalog_ingest::upsert_playlist_from_sc(&self.pg, payload, observation).await?)
    }

    pub async fn page_track_ids(
        &self,
        playlist_urn: &str,
        offset: i64,
        limit: i64,
    ) -> AppResult<Vec<String>> {
        let rows = sqlx::query_file_scalar!(
            "queries/playlists/page_track_ids.sql",
            playlist_urn,
            offset,
            limit
        )
        .fetch_all(&self.pg)
        .await?;
        Ok(rows)
    }
}

pub fn project_to_sc_shape(row: &PlaylistRow, owner: Option<&Value>) -> Value {
    let mut obj = row.sc_metadata.as_object().cloned().unwrap_or_default();
    obj.insert(
        "kind".into(),
        Value::String(row.kind.clone().unwrap_or_else(|| "playlist".into())),
    );
    obj.insert("urn".into(), Value::String(row.urn.clone()));
    obj.insert("id".into(), parse_id_or_string(&row.sc_playlist_id));
    obj.insert("title".into(), Value::String(row.title.clone()));
    if let Some(d) = &row.description {
        obj.insert("description".into(), Value::String(d.clone()));
    }
    if let Some(g) = &row.genre {
        obj.insert("genre".into(), Value::String(g.clone()));
    }
    obj.insert("tag_list".into(), Value::String(row.tags.join(" ")));
    if let Some(a) = &row.artwork_url {
        obj.insert("artwork_url".into(), Value::String(a.clone()));
    }
    if let Some(p) = &row.permalink_url {
        obj.insert("permalink_url".into(), Value::String(p.clone()));
    }
    obj.insert("track_count".into(), json!(row.track_count));
    if let Some(d) = row.duration_ms {
        obj.insert("duration".into(), json!(d));
    }
    obj.insert(
        "likes_count".into(),
        row.likes_count_sc.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    obj.insert(
        "reposts_count".into(),
        row.reposts_count_sc
            .map(|v| json!(v))
            .unwrap_or(Value::Null),
    );
    if let Some(p) = &row.playlist_type {
        obj.insert("playlist_type".into(), Value::String(p.clone()));
        if matches!(p.as_str(), "album" | "playlist") {
            obj.insert("set_type".into(), Value::String(p.clone()));
        }
    }
    obj.insert("sharing".into(), Value::String(row.sharing.clone()));
    if let Some(y) = row.release_year {
        obj.insert("release_year".into(), json!(y));
    }
    if let Some(d) = row.release_date {
        obj.insert("release_date".into(), Value::String(d.to_string()));
    }
    if let Some(l) = &row.label_name {
        obj.insert("label_name".into(), Value::String(l.clone()));
    }
    if let Some(t) = row.sc_created_at {
        obj.insert("created_at".into(), Value::String(t.to_rfc3339()));
    }
    if let Some(t) = row.sc_last_modified {
        obj.insert("last_modified".into(), Value::String(t.to_rfc3339()));
    }

    let owner_val = owner.cloned().unwrap_or_else(|| {
        let mut u = Map::new();
        u.insert("kind".into(), Value::String("user".into()));
        if let Some(id) = &row.owner_sc_user_id {
            u.insert("id".into(), parse_id_or_string(id));
        }
        if let Some(urn) = &row.owner_urn {
            u.insert("urn".into(), Value::String(urn.clone()));
        }
        if let Some(n) = &row.owner_username {
            u.insert("username".into(), Value::String(n.clone()));
        }
        Value::Object(u)
    });
    obj.insert("user".into(), owner_val);

    Value::Object(obj)
}
