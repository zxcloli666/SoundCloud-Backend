use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::Observation;
use crate::release_date;
use crate::sc_payload::{parse_dt, string_field};
use catalog_normalize::normalize_title;

pub async fn upsert_playlist_from_sc(
    pool: &PgPool,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let inserted = upsert_playlist_in(&mut transaction, payload, observation).await?;
    transaction.commit().await?;
    Ok(inserted)
}

pub async fn upsert_playlist_in(
    connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    let Some(fields) = ScPlaylistFields::from_sc(payload) else {
        return Ok(false);
    };
    sqlx::query_file_scalar!("queries/playlists/lock_membership.sql", &fields.urn)
        .fetch_optional(&mut **connection)
        .await?;
    let row = sqlx::query_file!(
        "queries/playlists/upsert.sql",
        &fields.urn,
        &fields.sc_playlist_id,
        &fields.title,
        &fields.title_normalized,
        fields.description.as_deref(),
        fields.genre.as_deref(),
        &fields.tags,
        fields.artwork_url.as_deref(),
        fields.permalink_url.as_deref(),
        fields.owner_sc_user_id.as_deref(),
        fields.owner_urn.as_deref(),
        fields.owner_username.as_deref(),
        fields.track_count,
        fields.duration_ms,
        fields.playlist_type.as_deref(),
        fields.kind.as_deref(),
        &fields.sharing,
        fields.release_year,
        fields.release_date,
        fields.label_name.as_deref(),
        fields.likes_count_sc,
        fields.reposts_count_sc,
        fields.sc_created_at,
        fields.sc_last_modified,
        observation.sequence(),
        &crate::playlist_metadata::metadata_from_sc(payload)
    )
    .fetch_optional(&mut **connection)
    .await?;
    sqlx::query_file!("queries/playlists/ensure_membership_state.sql", &fields.urn)
        .execute(&mut **connection)
        .await?;
    Ok(row.map(|r| r.was_new).unwrap_or(false))
}

struct ScPlaylistFields {
    urn: String,
    sc_playlist_id: String,
    title: String,
    title_normalized: String,
    description: Option<String>,
    genre: Option<String>,
    tags: Vec<String>,
    artwork_url: Option<String>,
    permalink_url: Option<String>,
    owner_sc_user_id: Option<String>,
    owner_urn: Option<String>,
    owner_username: Option<String>,
    track_count: i32,
    duration_ms: Option<i64>,
    playlist_type: Option<String>,
    kind: Option<String>,
    sharing: String,
    release_year: Option<i16>,
    release_date: Option<NaiveDate>,
    label_name: Option<String>,
    likes_count_sc: Option<i64>,
    reposts_count_sc: Option<i64>,
    sc_created_at: Option<DateTime<Utc>>,
    sc_last_modified: Option<DateTime<Utc>>,
}

impl ScPlaylistFields {
    fn from_sc(payload: &Value) -> Option<Self> {
        let urn = payload.get("urn").and_then(|v| v.as_str())?.to_string();
        if urn.is_empty() {
            return None;
        }
        let sc_playlist_id = crate::sc_ids::extract_sc_id(&urn).to_string();
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
        let owner_urn = owner
            .and_then(|u| u.get("urn"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let owner_sc_user_id = owner_urn
            .as_deref()
            .map(|u| crate::sc_ids::extract_sc_id(u).to_string())
            .or_else(|| {
                owner
                    .and_then(|u| u.get("id"))
                    .and_then(|v| v.as_i64())
                    .map(|i| i.to_string())
            });
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
