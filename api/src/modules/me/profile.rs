use backend_contracts::CatalogEntity;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::sc_ids::{extract_sc_id, user_id_variants, user_urn};
use crate::error::AppResult;
use crate::modules::cold_refresh::entity::enqueue_entity;

const PROFILE_TTL_SEC: i64 = 600;

pub async fn read(pool: &PgPool, sc_user_id: &str) -> AppResult<Value> {
    let variants = user_id_variants(sc_user_id);
    let row = sqlx::query_file!("queries/me/service/profile_cold_fetch.sql", &variants)
        .fetch_optional(pool)
        .await?;
    if let Some(row) = row {
        let age = Utc::now() - row.synced_at;
        if (age > Duration::seconds(PROFILE_TTL_SEC) || age < Duration::zero())
            && let Err(error) =
                enqueue_entity(pool, CatalogEntity::Profile, sc_user_id, Some(sc_user_id)).await
        {
            tracing::debug!(%error, "profile refresh enqueue deferred");
        }
        return Ok(row.profile_json);
    }
    enqueue_entity(pool, CatalogEntity::Profile, sc_user_id, Some(sc_user_id)).await?;
    stub(pool, sc_user_id).await
}

async fn stub(pool: &PgPool, sc_user_id: &str) -> AppResult<Value> {
    let sc_user_id = extract_sc_id(sc_user_id);
    let id = sc_user_id.parse::<i64>().unwrap_or_default();
    let mirrored =
        sqlx::query_file_scalar!("queries/me/service/stub_from_users.sql", sc_user_id, id)
            .fetch_optional(pool)
            .await?;
    if let Some(profile) = mirrored {
        return Ok(profile);
    }
    let variants = user_id_variants(sc_user_id);
    let username = sqlx::query_file_scalar!(
        "queries/me/service/stub_username_from_sessions.sql",
        &variants
    )
    .fetch_optional(pool)
    .await?;
    Ok(json!({
        "id": id, "urn": user_urn(sc_user_id), "username": username.unwrap_or_default(),
        "avatar_url": "", "permalink_url": "", "followers_count": 0, "followings_count": 0,
        "track_count": 0, "playlist_count": 0, "public_favorites_count": 0,
    }))
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
