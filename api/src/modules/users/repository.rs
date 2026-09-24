use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use sqlx::FromRow;
use sqlx::PgPool;

use crate::common::sc_payload::parse_id_or_string;
use crate::error::AppResult;

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)]
pub struct UserRow {
    pub sc_user_id: String,
    pub urn: String,
    pub username: String,
    pub username_normalized: String,
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub permalink: Option<String>,
    pub permalink_url: Option<String>,
    pub avatar_url: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub description: Option<String>,
    pub verified: bool,
    pub followers_count: Option<i64>,
    pub followings_count: Option<i64>,
    pub tracks_count: Option<i64>,
    pub playlists_count: Option<i64>,
    pub reposts_count: Option<i64>,
    pub comments_count: Option<i64>,
    pub kind: Option<String>,
    pub sc_created_at: Option<DateTime<Utc>>,
    pub sc_last_modified: Option<DateTime<Utc>>,
    pub sc_synced_at: DateTime<Utc>,
    pub last_read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct UserRepository {
    pg: PgPool,
}

impl UserRepository {
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    pub async fn find_by_urn(&self, urn: &str) -> AppResult<Option<UserRow>> {
        let canonical = if urn.contains(':') {
            urn.to_owned()
        } else {
            crate::common::sc_ids::user_urn(urn)
        };
        let row = sqlx::query_file_as!(
            UserRow,
            "queries/users/repository/find_by_urn.sql",
            &canonical
        )
        .fetch_optional(&self.pg)
        .await?;
        Ok(row)
    }

    pub async fn touch_last_read(&self, urn: &str) -> AppResult<()> {
        let canonical = if urn.contains(':') {
            urn.to_owned()
        } else {
            crate::common::sc_ids::user_urn(urn)
        };
        sqlx::query_file!("queries/users/repository/touch_last_read.sql", &canonical)
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    pub async fn upsert_from_sc(
        &self,
        payload: &Value,
        observation: catalog_ingest::Observation,
    ) -> AppResult<bool> {
        Ok(catalog_ingest::upsert_user_from_sc(&self.pg, payload, observation).await?)
    }
}

pub fn project_to_sc_shape(row: &UserRow) -> Value {
    let mut obj = Map::new();
    obj.insert("kind".into(), Value::String("user".into()));
    obj.insert("id".into(), parse_id_or_string(&row.sc_user_id));
    obj.insert("urn".into(), Value::String(row.urn.clone()));
    obj.insert("username".into(), Value::String(row.username.clone()));
    if let Some(n) = &row.full_name {
        obj.insert("full_name".into(), Value::String(n.clone()));
    }
    if let Some(n) = &row.first_name {
        obj.insert("first_name".into(), Value::String(n.clone()));
    }
    if let Some(n) = &row.last_name {
        obj.insert("last_name".into(), Value::String(n.clone()));
    }
    if let Some(p) = &row.permalink {
        obj.insert("permalink".into(), Value::String(p.clone()));
    }
    if let Some(p) = &row.permalink_url {
        obj.insert("permalink_url".into(), Value::String(p.clone()));
    }
    if let Some(a) = &row.avatar_url {
        obj.insert("avatar_url".into(), Value::String(a.clone()));
    }
    if let Some(c) = &row.country {
        obj.insert("country_code".into(), Value::String(c.clone()));
    }
    if let Some(c) = &row.city {
        obj.insert("city".into(), Value::String(c.clone()));
    }
    if let Some(d) = &row.description {
        obj.insert("description".into(), Value::String(d.clone()));
    }
    obj.insert("verified".into(), Value::Bool(row.verified));
    obj.insert(
        "followers_count".into(),
        row.followers_count.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    obj.insert(
        "followings_count".into(),
        row.followings_count
            .map(|v| json!(v))
            .unwrap_or(Value::Null),
    );
    obj.insert(
        "track_count".into(),
        row.tracks_count.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    obj.insert(
        "playlist_count".into(),
        row.playlists_count.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    obj.insert(
        "reposts_count".into(),
        row.reposts_count.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    obj.insert(
        "comments_count".into(),
        row.comments_count.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    if let Some(t) = row.sc_created_at {
        obj.insert("created_at".into(), Value::String(t.to_rfc3339()));
    }
    if let Some(t) = row.sc_last_modified {
        obj.insert("last_modified".into(), Value::String(t.to_rfc3339()));
    }
    Value::Object(obj)
}
