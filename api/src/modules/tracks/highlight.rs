#[cfg(test)]
#[path = "highlight_tests.rs"]
mod tests;

use std::collections::HashMap;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;

pub struct Highlight {
    pub start_ms: i32,
    pub length_ms: i32,
}

pub async fn by_track_ids(pg: &PgPool, track_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Highlight>> {
    if track_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_file!("queries/tracks/highlights/by_track_ids.sql", track_ids)
        .fetch_all(pg)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let highlight = Highlight {
                start_ms: row.start_ms,
                length_ms: row.length_ms,
            };
            (row.track_id, highlight)
        })
        .collect())
}

pub fn attach(track: &mut Value, highlight: Option<&Highlight>) {
    if let (Some(highlight), Some(fields)) = (highlight, track.as_object_mut()) {
        fields.insert(
            "highlight".into(),
            json!({"startMs": highlight.start_ms, "lengthMs": highlight.length_ms}),
        );
    }
}
