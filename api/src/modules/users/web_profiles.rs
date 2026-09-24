use backend_contracts::{CatalogEntity, CatalogRefreshPayload};
use serde_json::Value;
use sqlx::PgPool;

use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};
use crate::modules::cold_refresh::entity::enqueue_entity;

pub(super) async fn read(pool: &PgPool, user_urn: &str) -> AppResult<Value> {
    let payload = CatalogRefreshPayload {
        entity: CatalogEntity::WebProfiles,
        sc_id: extract_sc_id(user_urn).to_owned(),
        owner_id: None,
    };
    if !payload.is_valid()
        || (user_urn.contains(':') && user_urn != payload.entity.urn(&payload.sc_id))
    {
        return Err(AppError::bad_request("Invalid user resource"));
    }
    let snapshot = sqlx::query_file!("queries/users/service/web_profiles.sql", &payload.sc_id)
        .fetch_optional(pool)
        .await?;
    if snapshot.as_ref().is_none_or(|row| !row.fresh) {
        let enqueued = enqueue_entity(pool, payload.entity, user_urn, None).await;
        if snapshot.is_none() {
            enqueued?;
        } else if let Err(error) = enqueued {
            tracing::warn!(%error, "web profiles refresh enqueue failed");
        }
    }
    if let Some(snapshot) = snapshot {
        return Ok(snapshot.profiles);
    }
    let retry_after = sqlx::query_file_scalar!(
        "queries/cold_refresh/entity_retry_after.sql",
        payload.dedup_key()
    )
    .fetch_optional(pool)
    .await?
    .unwrap_or(5);
    Err(AppError::coded(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "web_profiles_refresh_pending",
        "Profile links are being loaded",
    )
    .with_retry_after(retry_after))
}

#[cfg(test)]
#[path = "web_profiles_tests.rs"]
mod tests;
