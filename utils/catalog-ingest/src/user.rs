use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::Observation;
use crate::sc_payload::{parse_dt, string_field};

pub async fn upsert_user_from_sc(
    pool: &PgPool,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let inserted = upsert_user_in(&mut transaction, payload, observation).await?;
    transaction.commit().await?;
    Ok(inserted)
}

pub async fn upsert_user_in(
    connection: &mut sqlx::PgConnection,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    let Some(fields) = ScUserFields::from_sc(payload) else {
        return Err(sqlx::Error::Protocol(
            "invalid SoundCloud user payload".into(),
        ));
    };
    let row = sqlx::query_file!(
        "queries/users/upsert.sql",
        &fields.sc_user_id,
        &fields.urn,
        &fields.username,
        &fields.username_normalized,
        fields.full_name.as_deref(),
        fields.first_name.as_deref(),
        fields.last_name.as_deref(),
        fields.permalink.as_deref(),
        fields.permalink_url.as_deref(),
        fields.avatar_url.as_deref(),
        fields.country.as_deref(),
        fields.city.as_deref(),
        fields.description.as_deref(),
        fields.verified,
        fields.followers_count,
        fields.followings_count,
        fields.tracks_count,
        fields.playlists_count,
        fields.reposts_count,
        fields.comments_count,
        fields.kind.as_deref(),
        fields.sc_created_at,
        fields.sc_last_modified,
        observation.sequence(),
        payload
    )
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.map(|r| r.was_new).unwrap_or(false))
}

pub async fn upsert_profile_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: &str,
    profile: &Value,
    observation: Observation,
) -> Result<(), sqlx::Error> {
    if profile.get("urn").and_then(Value::as_str) != Some(crate::sc_ids::user_urn(user_id).as_str())
    {
        return Err(sqlx::Error::Protocol(
            "SoundCloud profile identity mismatch".into(),
        ));
    }
    upsert_user_in(transaction, profile, observation).await?;
    sqlx::query_file!(
        "queries/users/upsert_profile.sql",
        user_id,
        profile,
        observation.sequence()
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

struct ScUserFields {
    sc_user_id: String,
    urn: String,
    username: String,
    username_normalized: String,
    full_name: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    permalink: Option<String>,
    permalink_url: Option<String>,
    avatar_url: Option<String>,
    country: Option<String>,
    city: Option<String>,
    description: Option<String>,
    verified: bool,
    followers_count: Option<i64>,
    followings_count: Option<i64>,
    tracks_count: Option<i64>,
    playlists_count: Option<i64>,
    reposts_count: Option<i64>,
    comments_count: Option<i64>,
    kind: Option<String>,
    sc_created_at: Option<DateTime<Utc>>,
    sc_last_modified: Option<DateTime<Utc>>,
}

impl ScUserFields {
    fn from_sc(payload: &Value) -> Option<Self> {
        let urn = payload.get("urn").and_then(|v| v.as_str())?.to_string();
        if urn.is_empty() {
            return None;
        }
        let sc_user_id = crate::sc_ids::extract_sc_id(&urn).to_string();
        let username = payload
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if username.trim().is_empty() {
            return None;
        }
        let username_normalized = catalog_normalize::normalize_name(&username);

        let full_name = string_field(payload, "full_name");
        let first_name = string_field(payload, "first_name");
        let last_name = string_field(payload, "last_name");
        let permalink = string_field(payload, "permalink");
        let permalink_url = string_field(payload, "permalink_url");
        let avatar_url = string_field(payload, "avatar_url");
        let country =
            string_field(payload, "country_code").or_else(|| string_field(payload, "country"));
        let city = string_field(payload, "city");
        let description = string_field(payload, "description");
        let verified = payload
            .get("verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let kind = string_field(payload, "kind");
        let sc_created_at = parse_dt(payload.get("created_at"));
        let sc_last_modified = parse_dt(payload.get("last_modified"));

        Some(Self {
            sc_user_id,
            urn,
            username,
            username_normalized,
            full_name,
            first_name,
            last_name,
            permalink,
            permalink_url,
            avatar_url,
            country,
            city,
            description,
            verified,
            followers_count: payload.get("followers_count").and_then(|v| v.as_i64()),
            followings_count: payload.get("followings_count").and_then(|v| v.as_i64()),
            tracks_count: payload.get("track_count").and_then(|v| v.as_i64()),
            playlists_count: payload.get("playlist_count").and_then(|v| v.as_i64()),
            reposts_count: payload.get("reposts_count").and_then(|v| v.as_i64()),
            comments_count: payload.get("comments_count").and_then(|v| v.as_i64()),
            kind,
            sc_created_at,
            sc_last_modified,
        })
    }
}
