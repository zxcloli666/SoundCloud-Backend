use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use wt::{Mode, ObserveProfile, ObserveUsers};

use crate::Observation;
use crate::sc_payload::{parse_dt, string_field, without_nul};

const REPLACED_WHEN_PRESENT: [&str; 12] = [
    "full_name",
    "first_name",
    "last_name",
    "permalink",
    "permalink_url",
    "avatar_url",
    "country",
    "country_code",
    "city",
    "description",
    "verified",
    "kind",
];

pub async fn upsert_user_from_sc(
    pool: &PgPool,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let inserted = upsert_user_in(&mut transaction, payload, observation).await?;
        transaction.commit().await?;
        Ok(inserted)
    })
    .await
}

pub async fn upsert_user_in(
    connection: &mut PgConnection,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    upsert_user_at(connection, payload, observation, wt::now()).await
}

pub(crate) async fn upsert_user_at(
    connection: &mut PgConnection,
    payload: &Value,
    observation: Observation,
    observed_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let payload = &without_nul(payload);
    let Some(fields) = ScUserFields::from_sc(payload) else {
        return Err(sqlx::Error::Protocol(
            "invalid SoundCloud user payload".into(),
        ));
    };
    let mut row = fields.row();
    row["keys"] = REPLACED_WHEN_PRESENT
        .iter()
        .filter(|key| payload.get(**key).is_some())
        .map(|key| Value::from(*key))
        .collect();
    let upsert = ObserveUsers {
        mode: Mode::Upsert,
        observed_at,
        observation: observation.sequence(),
        rows: vec![row],
    };
    Ok(wt::call(connection, &upsert)
        .await?
        .first()
        .is_some_and(|user| user.was_new))
}

pub async fn upsert_profile_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: &str,
    profile: &Value,
    observation: Observation,
) -> Result<(), sqlx::Error> {
    upsert_profile_at(transaction, user_id, profile, observation, wt::now()).await
}

pub(crate) async fn upsert_profile_at(
    connection: &mut PgConnection,
    user_id: &str,
    profile: &Value,
    observation: Observation,
    observed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let profile = &without_nul(profile);
    if profile.get("urn").and_then(Value::as_str) != Some(crate::sc_ids::user_urn(user_id).as_str())
    {
        return Err(sqlx::Error::Protocol(
            "SoundCloud profile identity mismatch".into(),
        ));
    }
    upsert_user_at(connection, profile, observation, observed_at).await?;
    let observed = ObserveProfile {
        user_id: user_id.to_owned(),
        profile: profile.clone(),
        observed_at,
        observation: observation.sequence(),
    };
    wt::call(connection, &observed).await
}

pub(crate) struct ScUserFields {
    pub(crate) sc_user_id: String,
    pub(crate) urn: String,
    pub(crate) username: String,
    pub(crate) username_normalized: String,
    pub(crate) full_name: Option<String>,
    pub(crate) first_name: Option<String>,
    pub(crate) last_name: Option<String>,
    pub(crate) permalink: Option<String>,
    pub(crate) permalink_url: Option<String>,
    pub(crate) avatar_url: Option<String>,
    pub(crate) country: Option<String>,
    pub(crate) city: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) verified: bool,
    pub(crate) followers_count: Option<i64>,
    pub(crate) followings_count: Option<i64>,
    pub(crate) tracks_count: Option<i64>,
    pub(crate) playlists_count: Option<i64>,
    pub(crate) reposts_count: Option<i64>,
    pub(crate) comments_count: Option<i64>,
    pub(crate) kind: Option<String>,
    pub(crate) sc_created_at: Option<DateTime<Utc>>,
    pub(crate) sc_last_modified: Option<DateTime<Utc>>,
}

impl ScUserFields {
    pub(crate) fn sc_user_id(&self) -> &str {
        &self.sc_user_id
    }

    pub(crate) fn row(&self) -> Value {
        serde_json::json!({
            "sc_user_id": self.sc_user_id,
            "urn": self.urn,
            "username": self.username,
            "username_normalized": self.username_normalized,
            "full_name": self.full_name,
            "first_name": self.first_name,
            "last_name": self.last_name,
            "permalink": self.permalink,
            "permalink_url": self.permalink_url,
            "avatar_url": self.avatar_url,
            "country": self.country,
            "city": self.city,
            "description": self.description,
            "verified": self.verified,
            "followers_count": self.followers_count,
            "followings_count": self.followings_count,
            "tracks_count": self.tracks_count,
            "playlists_count": self.playlists_count,
            "reposts_count": self.reposts_count,
            "comments_count": self.comments_count,
            "kind": self.kind,
            "sc_created_at": self.sc_created_at,
            "sc_last_modified": self.sc_last_modified,
        })
    }

    pub(crate) fn from_sc(payload: &Value) -> Option<Self> {
        let entity = crate::sc_ids::payload_ref(crate::EntityKind::User, payload)?;
        let urn = entity.urn();
        let sc_user_id = entity.sc_id();
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
