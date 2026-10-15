use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::common::sc_ids::{extract_sc_id, user_id_variants};
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize)]
pub struct SuperlikesPage {
    pub collection: Vec<Value>,
    pub next_href: Option<String>,
}

pub async fn page(
    pg: &PgPool,
    sc_user_id: &str,
    limit: i64,
    offset: i64,
) -> AppResult<SuperlikesPage> {
    let mut ids = sqlx::query_file_scalar!(
        "queries/superlikes/page.sql",
        &user_id_variants(sc_user_id),
        extract_sc_id(sc_user_id),
        limit + 1,
        offset
    )
    .fetch_all(pg)
    .await?;
    let has_more = ids.len() as i64 > limit;
    ids.truncate(limit as usize);
    Ok(SuperlikesPage {
        collection: project(pg, &ids).await?,
        next_href: has_more.then(|| format!("?limit={limit}&offset={}", offset + limit)),
    })
}

async fn project(pg: &PgPool, ids: &[String]) -> AppResult<Vec<Value>> {
    let mut tracks: Vec<Value> = crate::modules::tracks::project_many(pg, ids)
        .await?
        .into_iter()
        .flatten()
        .collect();
    crate::modules::enrich::dto::apply_to_tracks(pg, &mut tracks).await?;
    for track in tracks.iter_mut() {
        if let Some(object) = track.as_object_mut() {
            object.insert("user_favorite".into(), Value::Bool(true));
            object.insert("user_superlike".into(), Value::Bool(true));
        }
    }
    Ok(tracks)
}
