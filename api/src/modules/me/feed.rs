use serde_json::Value;
use sqlx::PgPool;

use crate::cache::ListPageResult;
use crate::common::sc_ids::{extract_sc_id, user_id_variants};
use crate::error::AppResult;
use crate::modules::likes::cold::apply_user_favorite_flag;

#[derive(serde::Serialize)]
pub struct FollowingsTracksPage {
    #[serde(flatten)]
    pub page: ListPageResult<Value>,
    #[serde(rename = "followingsSync")]
    pub followings_sync: crate::modules::cold_refresh::collection::CollectionSync,
}

pub(super) fn target_urn(input: &str) -> AppResult<String> {
    let id = input.strip_prefix("soundcloud:users:").unwrap_or(input);
    let payload = backend_contracts::CatalogCollectionPayload {
        collection: backend_contracts::CatalogCollection::OwnedTracks,
        subject_id: id.to_owned(),
        owner: false,
    };
    if !payload.is_valid() {
        return Err(crate::error::AppError::bad_request(
            "Invalid following account",
        ));
    }
    Ok(format!("soundcloud:users:{id}"))
}

pub(super) async fn read(
    pool: &PgPool,
    user_id: &str,
    page: i64,
    limit: i64,
) -> AppResult<ListPageResult<Value>> {
    let page = page.clamp(0, 24);
    let limit = limit.clamp(1, 50);
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = '2500ms'")
        .execute(&mut *tx)
        .await?;
    let keys = sqlx::query_file_scalar!(
        "queries/me/service/followings_tracks.sql",
        &user_id_variants(user_id),
        extract_sc_id(user_id),
        limit + 1,
        page * limit
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let has_more = keys.len() > limit as usize && page < 24;
    let keys: Vec<_> = keys.into_iter().take(limit as usize).collect();
    let mut collection: Vec<_> = crate::modules::tracks::project_many_public(pool, &keys)
        .await?
        .into_iter()
        .flatten()
        .collect();
    apply_user_favorite_flag(pool, user_id, &mut collection).await?;
    Ok(ListPageResult {
        collection,
        page,
        page_size: limit,
        has_more,
    })
}

#[cfg(test)]
#[path = "feed_tests.rs"]
mod tests;
