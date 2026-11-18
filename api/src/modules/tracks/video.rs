#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::error::AppResult;

static BASE_URL: OnceLock<String> = OnceLock::new();

pub struct Video {
    pub kind: String,
    pub s3_key: String,
    pub offset_ms: i32,
    pub looped: bool,
}

pub fn set_base_url(url: &str) {
    let _ = BASE_URL.set(url.trim_end_matches('/').to_string());
}

pub async fn by_sc_track_ids(
    pg: &PgPool,
    sc_track_ids: &[String],
) -> AppResult<HashMap<String, Vec<Video>>> {
    let mut by_track: HashMap<String, Vec<Video>> = HashMap::new();
    if sc_track_ids.is_empty() {
        return Ok(by_track);
    }
    let rows = sqlx::query_file!("queries/tracks/videos/by_sc_track_ids.sql", sc_track_ids)
        .fetch_all(pg)
        .await?;
    for row in rows {
        let video = Video {
            kind: row.kind,
            s3_key: row.s3_key,
            offset_ms: row.offset_ms,
            looped: row.looped,
        };
        by_track.entry(row.sc_track_id).or_default().push(video);
    }
    Ok(by_track)
}

pub fn attach(track: &mut Value, videos: Option<&Vec<Video>>) {
    let (Some(videos), Some(base), Some(fields)) = (videos, BASE_URL.get(), track.as_object_mut())
    else {
        return;
    };
    if videos.is_empty() {
        return;
    }
    let entries: Vec<Value> = videos
        .iter()
        .map(|video| {
            json!({
                "kind": video.kind,
                "url": format!("{base}/{}", video.s3_key),
                "offsetMs": video.offset_ms,
                "loop": video.looped,
            })
        })
        .collect();
    fields.insert("videos".into(), Value::Array(entries));
}
