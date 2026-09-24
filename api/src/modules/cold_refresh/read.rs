use serde_json::Value;
use sqlx::PgPool;

use crate::cache::ListPageResult;
use crate::error::{AppError, AppResult};
use crate::modules::users::project_to_sc_shape as project_user;

use super::service::{AudienceCollection, EntityKind, UserCollection};

async fn project_users(pg: &PgPool, urns: &[String]) -> AppResult<Vec<Value>> {
    let rows: Vec<crate::modules::users::UserRow> = sqlx::query_file_as!(
        crate::modules::users::UserRow,
        "queries/cold_refresh/service/users_by_urns.sql",
        urns
    )
    .fetch_all(pg)
    .await?;
    let map: std::collections::HashMap<String, crate::modules::users::UserRow> =
        rows.into_iter().map(|row| (row.urn.clone(), row)).collect();
    Ok(urns
        .iter()
        .filter_map(|urn| map.get(urn).map(project_user))
        .collect())
}

pub async fn read_audience_page(
    pg: &PgPool,
    coll: &AudienceCollection,
    subject_urn: &str,
    page: i64,
    limit: i64,
) -> AppResult<ListPageResult<Value>> {
    if !(1..=200).contains(&limit) {
        return Err(AppError::bad_request(
            "Collection page size must be between 1 and 200",
        ));
    }
    let page = page.clamp(0, 100);
    let offset = page
        .checked_mul(limit)
        .ok_or_else(|| AppError::bad_request("Collection page is out of range"))?;
    let keys = sqlx::query_file_scalar!(
        "queries/cold_refresh/audience_page.sql",
        coll.subject_urn(subject_urn),
        coll.kind.as_str(),
        limit + 1,
        offset
    )
    .fetch_all(pg)
    .await?;
    let has_more = page < 100 && keys.len() as i64 > limit;
    let page_keys: Vec<String> = keys.into_iter().take(limit as usize).collect();
    Ok(ListPageResult {
        collection: project_users(pg, &page_keys).await?,
        page,
        page_size: limit,
        has_more,
    })
}

pub(super) async fn collection_page_keys(
    pg: &PgPool,
    coll: &UserCollection,
    sc_user_id: &str,
    page: i64,
    limit: i64,
    public_only: bool,
) -> AppResult<Vec<String>> {
    if !(1..=200).contains(&limit) {
        return Err(AppError::bad_request(
            "Collection page size must be between 1 and 200",
        ));
    }
    let offset = page
        .max(0)
        .checked_mul(limit)
        .ok_or_else(|| AppError::bad_request("Collection page is out of range"))?;
    if coll.kind == backend_contracts::CatalogCollection::Followers {
        return Ok(sqlx::query_file_scalar!(
            "queries/cold_refresh/followers_page.sql",
            crate::common::sc_ids::extract_sc_id(sc_user_id),
            limit + 1,
            offset
        )
        .fetch_all(pg)
        .await?);
    }
    let table = coll.mirror_table;
    let key_col = coll.mirror_key_col;
    let wanted_column = if coll.has_wanted_state {
        "wanted_state"
    } else {
        "true AS wanted_state"
    };
    let (entity_table, entity_key) = match coll.entity_kind {
        EntityKind::Track => ("tracks", "sc_track_id"),
        EntityKind::Playlist => ("playlists", "urn"),
        EntityKind::User => ("users", "urn"),
    };
    let visibility_filter = if public_only && !matches!(coll.entity_kind, EntityKind::User) {
        "AND e.sharing = 'public'"
    } else {
        ""
    };
    let deletion_filter = if matches!(coll.entity_kind, EntityKind::Track | EntityKind::Playlist) {
        "AND e.deleted_at IS NULL"
    } else {
        ""
    };
    let release_order = if coll.order_by_release {
        "e.release_date DESC NULLS LAST, e.sc_created_at DESC NULLS LAST,"
    } else {
        ""
    };
    let entity_columns = if coll.order_by_release {
        "e.release_date, e.sc_created_at"
    } else {
        "1 AS present"
    };
    let sql = format!(
        "SELECT m.{key_col} FROM (
             SELECT d.* FROM (
                 SELECT DISTINCT ON ({key_col}) {key_col}, created_at, {wanted_column}
                 FROM {table} WHERE user_id = ANY($1)
                 ORDER BY {key_col}, (user_id = $4) DESC
             ) d WHERE wanted_state
             ORDER BY created_at DESC, {key_col} DESC OFFSET 0
         ) m
         CROSS JOIN LATERAL (
             SELECT {entity_columns} FROM {entity_table} e
             WHERE e.{entity_key} = m.{key_col} {visibility_filter} {deletion_filter} OFFSET 0
         ) e
         ORDER BY {release_order} m.created_at DESC, m.{key_col} DESC
         LIMIT $2 OFFSET $3"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(crate::common::sc_ids::user_id_variants(sc_user_id))
        .bind(limit + 1)
        .bind(offset)
        .bind(crate::common::sc_ids::extract_sc_id(sc_user_id))
        .fetch_all(pg)
        .await?)
}

pub async fn read_collection_page(
    pg: &PgPool,
    coll: &UserCollection,
    sc_user_id: &str,
    page: i64,
    limit: i64,
    public_only: bool,
) -> AppResult<ListPageResult<Value>> {
    let page = page.clamp(0, 100);
    let keys = collection_page_keys(pg, coll, sc_user_id, page, limit, public_only).await?;
    let has_more = page < 100 && keys.len() as i64 > limit;
    let page_keys: Vec<String> = keys.into_iter().take(limit as usize).collect();

    let collection: Vec<Value> = match coll.entity_kind {
        EntityKind::Track => {
            let projected = if public_only {
                crate::modules::tracks::project_many_public(pg, &page_keys).await?
            } else {
                crate::modules::tracks::project_many(pg, &page_keys).await?
            };
            projected.into_iter().flatten().collect()
        }
        EntityKind::User => project_users(pg, &page_keys).await?,
        EntityKind::Playlist => {
            let rows: Vec<crate::modules::playlists::PlaylistRow> = if public_only {
                sqlx::query_file_as!(
                    crate::modules::playlists::PlaylistRow,
                    "queries/cold_refresh/service/playlists_by_urns_public.sql",
                    &page_keys
                )
                .fetch_all(pg)
                .await?
            } else {
                sqlx::query_file_as!(
                    crate::modules::playlists::PlaylistRow,
                    "queries/cold_refresh/service/playlists_by_urns.sql",
                    &page_keys
                )
                .fetch_all(pg)
                .await?
            };
            let map: std::collections::HashMap<String, crate::modules::playlists::PlaylistRow> =
                rows.into_iter().map(|p| (p.urn.clone(), p)).collect();
            page_keys
                .iter()
                .filter_map(|urn| map.get(urn))
                .map(|p| crate::modules::playlists::project_to_sc_shape(p, None))
                .collect()
        }
    };

    Ok(ListPageResult {
        collection,
        page,
        page_size: limit,
        has_more,
    })
}
