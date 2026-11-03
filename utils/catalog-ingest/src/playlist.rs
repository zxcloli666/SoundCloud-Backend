use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use wt::{Mode, ObservePlaylists};

use crate::Observation;
use crate::release_date;
use crate::sc_payload::{parse_dt, string_field, without_nul};
use catalog_normalize::normalize_title;

pub async fn upsert_playlist_from_sc(
    pool: &PgPool,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let inserted = upsert_playlist_in(&mut transaction, payload, observation).await?;
        transaction.commit().await?;
        Ok(inserted)
    })
    .await
}

pub async fn upsert_playlist_in(
    connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    upsert_playlist_at(connection, payload, observation, wt::now()).await
}

pub(crate) async fn upsert_playlist_at(
    connection: &mut PgConnection,
    payload: &Value,
    observation: Observation,
    observed_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let payload = &without_nul(payload);
    let Some(fields) = ScPlaylistFields::from_sc(payload) else {
        return Ok(false);
    };
    let upsert = ObservePlaylists {
        mode: Mode::Upsert,
        observed_at,
        observation: observation.sequence(),
        rows: vec![fields.row(payload)],
    };
    Ok(wt::call(connection, &upsert)
        .await?
        .first()
        .is_some_and(|playlist| playlist.was_new))
}

pub(crate) struct ScPlaylistFields {
    pub(crate) urn: String,
    pub(crate) sc_playlist_id: String,
    pub(crate) title: String,
    pub(crate) title_normalized: String,
    pub(crate) description: Option<String>,
    pub(crate) genre: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) artwork_url: Option<String>,
    pub(crate) permalink_url: Option<String>,
    pub(crate) owner_sc_user_id: Option<String>,
    pub(crate) owner_urn: Option<String>,
    pub(crate) owner_username: Option<String>,
    pub(crate) track_count: i32,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) playlist_type: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) sharing: String,
    pub(crate) release_year: Option<i16>,
    pub(crate) release_date: Option<NaiveDate>,
    pub(crate) label_name: Option<String>,
    pub(crate) likes_count_sc: Option<i64>,
    pub(crate) reposts_count_sc: Option<i64>,
    pub(crate) sc_created_at: Option<DateTime<Utc>>,
    pub(crate) sc_last_modified: Option<DateTime<Utc>>,
}

impl ScPlaylistFields {
    pub(crate) fn urn(&self) -> &str {
        &self.urn
    }

    pub(crate) fn row(&self, payload: &Value) -> Value {
        serde_json::json!({
            "urn": self.urn,
            "sc_playlist_id": self.sc_playlist_id,
            "title": self.title,
            "title_normalized": self.title_normalized,
            "description": self.description,
            "genre": self.genre,
            "tags": self.tags,
            "artwork_url": self.artwork_url,
            "permalink_url": self.permalink_url,
            "owner_sc_user_id": self.owner_sc_user_id,
            "owner_urn": self.owner_urn,
            "owner_username": self.owner_username,
            "track_count": self.track_count,
            "duration_ms": self.duration_ms,
            "playlist_type": self.playlist_type,
            "kind": self.kind,
            "sharing": self.sharing,
            "release_year": self.release_year,
            "release_date": self.release_date,
            "label_name": self.label_name,
            "likes_count_sc": self.likes_count_sc,
            "reposts_count_sc": self.reposts_count_sc,
            "sc_created_at": self.sc_created_at,
            "sc_last_modified": self.sc_last_modified,
            "sc_metadata": crate::playlist_metadata::metadata_from_sc(payload),
        })
    }

    pub(crate) fn from_sc(payload: &Value) -> Option<Self> {
        let entity = crate::sc_ids::payload_ref(crate::EntityKind::Playlist, payload)?;
        let urn = entity.urn();
        let sc_playlist_id = entity.sc_id();
        let title = payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if title.is_empty() {
            return None;
        }
        let title_normalized = normalize_title(&title);

        let description = string_field(payload, "description");
        let genre = string_field(payload, "genre");
        let tag_list = payload
            .get("tag_list")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tags = tag_list
            .split_whitespace()
            .map(String::from)
            .filter(|s| !s.is_empty())
            .collect();

        let artwork_url = string_field(payload, "artwork_url");
        let permalink_url = string_field(payload, "permalink_url");

        let owner = payload.get("user");
        let owner_ref = owner.and_then(crate::sc_ids::user_ref);
        let owner_urn = owner_ref.map(crate::EntityRef::urn);
        let owner_sc_user_id = owner_ref.map(crate::EntityRef::sc_id);
        let owner_username = owner
            .and_then(|u| u.get("username"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let track_count = payload
            .get("track_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let duration_ms = payload.get("duration").and_then(|v| v.as_i64());
        let playlist_type = string_field(payload, "set_type")
            .or_else(|| string_field(payload, "playlist_type"))
            .or_else(|| string_field(payload, "type"));
        let kind = string_field(payload, "kind");
        let sharing = string_field(payload, "sharing").unwrap_or_else(|| "public".into());
        let label_name = string_field(payload, "label_name");
        let likes_count_sc = payload.get("likes_count").and_then(|v| v.as_i64());
        let reposts_count_sc = payload
            .get("reposts_count")
            .or_else(|| payload.get("repost_count"))
            .and_then(|v| v.as_i64());

        let (release_year, release_date) = release_date::extract(payload);
        let sc_created_at = parse_dt(payload.get("created_at"));
        let sc_last_modified = parse_dt(payload.get("last_modified"));

        Some(Self {
            urn,
            sc_playlist_id,
            title,
            title_normalized,
            description,
            genre,
            tags,
            artwork_url,
            permalink_url,
            owner_sc_user_id,
            owner_urn,
            owner_username,
            track_count,
            duration_ms,
            playlist_type,
            kind,
            sharing,
            release_year,
            release_date,
            label_name,
            likes_count_sc,
            reposts_count_sc,
            sc_created_at,
            sc_last_modified,
        })
    }
}
