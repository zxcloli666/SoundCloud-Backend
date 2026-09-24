use std::collections::HashSet;

use serde_json::Value;
use sqlx::PgPool;

use crate::common::sc_ids::extract_sc_id;
use crate::error::AppResult;

async fn fetch_liked_playlist_urns(
    pg: &PgPool,
    sc_user_id: &str,
    urns: &[String],
) -> AppResult<HashSet<String>> {
    if urns.is_empty() {
        return Ok(HashSet::new());
    }
    let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
    let rows = sqlx::query_file_scalar!(
        "queries/likes/cold/fetch_liked_playlist_urns.sql",
        &variants,
        urns,
    )
    .fetch_all(pg)
    .await?;
    Ok(rows.into_iter().collect())
}

pub async fn apply_user_favorite_flag_to_playlists(
    pg: &PgPool,
    sc_user_id: &str,
    playlists: &mut [Value],
) -> AppResult<()> {
    let urns: Vec<String> = playlists
        .iter()
        .filter_map(|p| p.get("urn").and_then(|v| v.as_str()).map(String::from))
        .collect();
    if urns.is_empty() {
        return Ok(());
    }
    let liked = fetch_liked_playlist_urns(pg, sc_user_id, &urns).await?;
    if liked.is_empty() {
        return Ok(());
    }
    for p in playlists.iter_mut() {
        let urn = p.get("urn").and_then(|v| v.as_str()).unwrap_or("");
        if liked.contains(urn)
            && let Some(obj) = p.as_object_mut()
        {
            obj.insert("user_favorite".into(), Value::Bool(true));
        }
    }
    Ok(())
}

async fn fetch_liked_ids(
    pg: &PgPool,
    sc_user_id: &str,
    sc_track_ids: &[String],
) -> AppResult<HashSet<String>> {
    if sc_track_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
    let rows = sqlx::query_file_scalar!(
        "queries/likes/cold/fetch_liked_ids.sql",
        &variants,
        sc_track_ids,
    )
    .fetch_all(pg)
    .await?;
    Ok(rows.into_iter().collect())
}

pub async fn apply_user_favorite_flag(
    pg: &PgPool,
    sc_user_id: &str,
    tracks: &mut [Value],
) -> AppResult<()> {
    let ids: Vec<String> = tracks
        .iter()
        .filter_map(|t| t.get("urn").and_then(|v| v.as_str()).map(extract_sc_id))
        .map(String::from)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let liked_ids = fetch_liked_ids(pg, sc_user_id, &ids).await?;
    if liked_ids.is_empty() {
        return Ok(());
    }
    for t in tracks.iter_mut() {
        let liked = t
            .get("urn")
            .and_then(|v| v.as_str())
            .is_some_and(|u| liked_ids.contains(extract_sc_id(u)));
        if liked && let Some(obj) = t.as_object_mut() {
            obj.insert("user_favorite".into(), Value::Bool(true));
        }
    }
    Ok(())
}
