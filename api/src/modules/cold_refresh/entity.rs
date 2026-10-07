use backend_contracts::{CatalogEntity, CatalogRefreshPayload, JobKind, Versioned};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};

pub async fn enqueue_entity(
    pool: &PgPool,
    entity: CatalogEntity,
    urn: &str,
    owner_id: Option<&str>,
) -> AppResult<()> {
    let mut connection = pool.acquire().await?;
    enqueue_entity_in(&mut connection, entity, urn, owner_id).await
}

pub async fn want_track(
    pool: &PgPool,
    sc_track_id: &str,
    priority: catalog_ingest::TrackPriority,
) -> AppResult<()> {
    let wanted = [sc_track_id.to_owned()];
    let missing =
        sqlx::query_file_scalar!("queries/playlists/missing_catalog_tracks.sql", &wanted[..])
            .fetch_all(pool)
            .await?;
    if missing.is_empty() {
        catalog_ingest::bump_track_priority(pool, sc_track_id, priority).await?;
        return Ok(());
    }
    enqueue_entity(pool, CatalogEntity::Track, sc_track_id, None).await
}

pub async fn refresh_pending(
    pool: &PgPool,
    entity: CatalogEntity,
    urn: &str,
    owner_id: Option<&str>,
    code: &'static str,
    message: &'static str,
) -> AppError {
    if let Err(error) = enqueue_entity(pool, entity, urn, owner_id).await {
        return error;
    }
    let payload = CatalogRefreshPayload {
        entity,
        sc_id: extract_sc_id(urn).to_owned(),
        owner_id: owner_id.map(extract_sc_id).map(str::to_owned),
    };
    let retry_after = sqlx::query_file_scalar!(
        "queries/cold_refresh/entity_retry_after.sql",
        payload.dedup_key()
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(5);
    AppError::coded(axum::http::StatusCode::SERVICE_UNAVAILABLE, code, message)
        .with_retry_after(retry_after)
}

pub async fn enqueue_entity_in(
    connection: &mut sqlx::PgConnection,
    entity: CatalogEntity,
    urn: &str,
    owner_id: Option<&str>,
) -> AppResult<()> {
    let payload = CatalogRefreshPayload {
        entity,
        sc_id: extract_sc_id(urn).to_owned(),
        owner_id: owner_id.map(extract_sc_id).map(str::to_owned),
    };
    if !payload.is_valid() || (urn.contains(':') && urn != entity.urn(&payload.sc_id)) {
        return Err(AppError::bad_request("Invalid catalog resource"));
    }
    let dedup_key = payload.dedup_key();
    let body = serde_json::to_value(Versioned::V1(payload))
        .map_err(|error| AppError::internal(error.to_string()))?;
    let kind = JobKind::CatalogRefresh;
    sqlx::query_file!(
        "queries/cold_refresh/enqueue_entity.sql",
        Uuid::now_v7(),
        kind.as_str(),
        kind.lane().as_str(),
        dedup_key,
        body
    )
    .execute(connection)
    .await?;
    Ok(())
}
