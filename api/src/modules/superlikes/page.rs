use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::common::sc_ids::user_id_variants;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
pub struct SuperlikesPage {
    pub collection: Vec<Value>,
    pub next_href: Option<String>,
}

pub async fn page(
    pg: &PgPool,
    sc_user_id: &str,
    limit: i64,
    cursor: Option<&str>,
) -> AppResult<SuperlikesPage> {
    let mut rows = superliked_rows(pg, sc_user_id, limit + 1, cursor).await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_href = rows
        .last()
        .filter(|_| has_more)
        .map(|(_, at)| next_href(limit, *at));
    let ids: Vec<String> = rows.into_iter().map(|(id, _)| id).collect();
    Ok(SuperlikesPage {
        collection: project(pg, &ids).await?,
        next_href,
    })
}

async fn superliked_rows(
    pg: &PgPool,
    sc_user_id: &str,
    limit: i64,
    cursor: Option<&str>,
) -> AppResult<Vec<(String, DateTime<Utc>)>> {
    let variants = user_id_variants(sc_user_id);
    let rows = match cursor {
        Some(cursor) => sqlx::query_file!(
            "queries/superlikes/page_after.sql",
            &variants,
            parse_cursor(cursor)?,
            limit
        )
        .fetch_all(pg)
        .await?
        .into_iter()
        .map(|row| (row.sc_track_id, row.created_at))
        .collect(),
        None => sqlx::query_file!("queries/superlikes/page.sql", &variants, limit)
            .fetch_all(pg)
            .await?
            .into_iter()
            .map(|row| (row.sc_track_id, row.created_at))
            .collect(),
    };
    Ok(rows)
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

fn next_href(limit: i64, last: DateTime<Utc>) -> String {
    let cursor = last.to_rfc3339_opts(SecondsFormat::Micros, true);
    format!("?limit={limit}&cursor={cursor}")
}

fn parse_cursor(raw: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|e| AppError::bad_request(format!("invalid cursor: {e}")))
}
