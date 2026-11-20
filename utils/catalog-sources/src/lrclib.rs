use std::sync::Arc;

use serde::Deserialize;
use wreq::header::{ACCEPT, HeaderMap, HeaderValue};

use crate::{ExternalFetcher, SourceError, SourceResult};

const SEARCH_URL: &str = "https://lrclib.net/api/search";
const MAX_RECORDS: usize = 12;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_REPLY_TEXT_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LrclibRecord {
    pub id: i64,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub duration_sec: Option<i64>,
    pub synced_lrc: Option<String>,
    pub plain_text: Option<String>,
}

#[derive(Deserialize)]
struct RawRecord {
    id: i64,
    #[serde(default, rename = "trackName")]
    track_name: Option<String>,
    #[serde(default, rename = "artistName")]
    artist_name: Option<String>,
    #[serde(default, rename = "albumName")]
    album_name: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default, rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(default, rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

#[derive(Clone)]
pub struct LrclibSearch {
    fetcher: Arc<ExternalFetcher>,
}

impl LrclibSearch {
    pub fn new(fetcher: Arc<ExternalFetcher>) -> Self {
        Self { fetcher }
    }

    pub async fn search(
        &self,
        query: &str,
        duration_sec: Option<i64>,
    ) -> SourceResult<Vec<LrclibRecord>> {
        let url = format!("{SEARCH_URL}?q={}", urlencoding::encode(query.trim()));
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let body = self.fetcher.get_bytes(&url, headers).await?;
        records_from(&body, duration_sec)
    }
}

pub fn records_from(body: &[u8], duration_sec: Option<i64>) -> SourceResult<Vec<LrclibRecord>> {
    let raw: Vec<RawRecord> = serde_json::from_slice(body)
        .map_err(|error| SourceError::Invalid(format!("lrclib search: {error}")))?;
    let mut records: Vec<LrclibRecord> = raw.into_iter().filter_map(record).collect();
    records.sort_by_key(|record| {
        (
            record.synced_lrc.is_none(),
            distance(duration_sec, record.duration_sec),
        )
    });
    records.truncate(MAX_RECORDS);
    let mut total = 0;
    records.retain(|record| {
        total += record.synced_lrc.as_ref().map_or(0, String::len)
            + record.plain_text.as_ref().map_or(0, String::len);
        total <= MAX_REPLY_TEXT_BYTES
    });
    Ok(records)
}

fn record(raw: RawRecord) -> Option<LrclibRecord> {
    let synced_lrc = text(raw.synced_lyrics);
    let plain_text = text(raw.plain_lyrics);
    if raw.id <= 0 || (synced_lrc.is_none() && plain_text.is_none()) {
        return None;
    }
    Some(LrclibRecord {
        id: raw.id,
        artist: raw.artist_name.unwrap_or_default().trim().to_owned(),
        title: raw.track_name.unwrap_or_default().trim().to_owned(),
        album: text(raw.album_name),
        duration_sec: raw
            .duration
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(|seconds| seconds.round() as i64),
        synced_lrc,
        plain_text,
    })
}

fn text(value: Option<String>) -> Option<String> {
    let value = value?;
    let value = value.trim();
    (!value.is_empty() && value.len() <= MAX_TEXT_BYTES && !value.contains('\0'))
        .then(|| value.to_owned())
}

fn distance(target: Option<i64>, candidate: Option<i64>) -> i64 {
    match (target, candidate) {
        (Some(target), Some(candidate)) => (target - candidate).abs(),
        _ => i64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synced_records_come_first_then_the_closest_length() {
        let body = br#"[
            {"id": 1, "trackName": "Song", "artistName": "A", "duration": 200, "plainLyrics": "plain words"},
            {"id": 2, "trackName": "Song", "artistName": "A", "duration": 300, "syncedLyrics": "[00:01.00] far"},
            {"id": 3, "trackName": "Song", "artistName": "A", "duration": 201.4, "syncedLyrics": "[00:01.00] near", "plainLyrics": "near"}
        ]"#;

        let ids: Vec<i64> = records_from(body, Some(200))
            .unwrap()
            .iter()
            .map(|record| record.id)
            .collect();

        assert_eq!(ids, vec![3, 2, 1]);
    }

    #[test]
    fn the_reply_stops_before_it_outgrows_a_bus_message() {
        let big = "y".repeat(MAX_TEXT_BYTES - 16);
        let rows: Vec<String> = (1..=12)
            .map(|id| format!(r#"{{"id": {id}, "trackName": "T", "plainLyrics": "{big}"}}"#))
            .collect();
        let body = format!("[{}]", rows.join(","));

        let records = records_from(body.as_bytes(), None).unwrap();

        assert_eq!(records.len(), MAX_REPLY_TEXT_BYTES / big.len());
    }

    #[test]
    fn instrumental_and_oversized_records_are_left_out() {
        let huge = "x".repeat(MAX_TEXT_BYTES + 1);
        let body = format!(
            r#"[{{"id": 1, "trackName": "Tune", "instrumental": true}},
                {{"id": 2, "trackName": "Long", "plainLyrics": "{huge}"}},
                {{"id": 3, "trackName": "Kept", "albumName": " ", "plainLyrics": "  words  "}}]"#
        );

        let records = records_from(body.as_bytes(), None).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, 3);
        assert_eq!(records[0].plain_text.as_deref(), Some("words"));
        assert_eq!(records[0].album, None);
    }
}
