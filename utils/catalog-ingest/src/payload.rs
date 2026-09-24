use serde_json::Value;

use crate::release_date;

#[derive(Debug, Clone)]
pub struct ScTrackFields {
    pub sc_track_id: String,
    pub urn: String,

    pub title: String,
    pub title_normalized: String,
    pub description: Option<String>,
    pub genre: Option<String>,
    pub tags: Vec<String>,
    pub duration_ms: i32,
    pub artwork_url: Option<String>,
    pub permalink_url: Option<String>,
    pub waveform_url: Option<String>,
    pub language: Option<String>,
    pub isrc: Option<String>,
    pub metadata_artist: Option<String>,
    pub sharing: String,
    pub sc_metadata: Value,
    pub sc_created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub sc_last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub release_year: Option<i16>,
    pub release_date: Option<chrono::NaiveDate>,

    pub uploader_sc_user_id: Option<String>,
    pub uploader_urn: Option<String>,
    pub uploader_username: Option<String>,
    pub uploader_avatar_url: Option<String>,

    pub play_count_sc: Option<i64>,
    pub likes_count_sc: Option<i64>,
    pub reposts_count_sc: Option<i64>,
    pub comments_count_sc: Option<i64>,

    pub needs_duration_resolve: bool,

    pub is_cover: bool,
}

impl ScTrackFields {
    pub fn from_sc(payload: &Value) -> Option<Self> {
        let urn = payload.get("urn").and_then(|v| v.as_str())?.to_string();
        if urn.is_empty() {
            return None;
        }
        let sc_track_id = crate::sc_ids::extract_sc_id(&urn).to_string();

        let raw_title = payload.get("title").and_then(|v| v.as_str()).unwrap_or("");
        if raw_title.is_empty() {
            return None;
        }
        let title = catalog_normalize::unescape_json_unicode(raw_title);
        let title_normalized = normalize_title(&title);
        let is_cover = catalog_normalize::title_marks_cover(&title);

        let description = string_field(payload, "description");
        let genre = string_field(payload, "genre");
        let tag_list = payload
            .get("tag_list")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tags = tag_list
            .split_whitespace()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();

        let full = payload.get("full_duration").and_then(|v| v.as_i64());
        let dur = payload.get("duration").and_then(|v| v.as_i64());
        let duration_ms = full.or(dur).unwrap_or(0) as i32;
        let needs_duration_resolve = duration_ms <= 0 || (duration_ms == 30000 && full.is_none());

        let artwork_url = string_field(payload, "artwork_url");
        let permalink_url = string_field(payload, "permalink_url");
        let waveform_url = string_field(payload, "waveform_url");
        let language = string_field(payload, "language");
        let isrc = extract_isrc(payload);
        let metadata_artist = string_field(payload, "metadata_artist")
            .map(|s| catalog_normalize::unescape_json_unicode(&s));
        let sharing = string_field(payload, "sharing").unwrap_or_else(|| "public".into());

        let sc_created_at = parse_dt(payload.get("created_at"));
        let sc_last_modified = parse_dt(payload.get("last_modified"));
        let (release_year, release_date) = release_date::extract(payload);

        let user = payload.get("user");
        let uploader_urn = user
            .and_then(|u| u.get("urn"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let uploader_sc_user_id = uploader_urn
            .as_deref()
            .map(|u| crate::sc_ids::extract_sc_id(u).to_string())
            .or_else(|| {
                user.and_then(|u| u.get("id"))
                    .and_then(|v| v.as_i64())
                    .map(|i| i.to_string())
            });
        let uploader_username = user
            .and_then(|u| u.get("username"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let uploader_avatar_url = user
            .and_then(|u| u.get("avatar_url"))
            .and_then(|v| v.as_str())
            .map(String::from);

        Some(Self {
            sc_track_id,
            urn,
            title,
            title_normalized,
            description,
            genre,
            tags,
            duration_ms,
            artwork_url,
            permalink_url,
            waveform_url,
            language,
            isrc,
            metadata_artist,
            sharing,
            sc_metadata: crate::track_metadata::metadata_from_sc(payload),
            sc_created_at,
            sc_last_modified,
            release_year,
            release_date,
            uploader_sc_user_id,
            uploader_urn,
            uploader_username,
            uploader_avatar_url,
            play_count_sc: payload.get("playback_count").and_then(|v| v.as_i64()),
            likes_count_sc: payload.get("likes_count").and_then(|v| v.as_i64()),
            reposts_count_sc: payload.get("reposts_count").and_then(|v| v.as_i64()),
            comments_count_sc: payload.get("comment_count").and_then(|v| v.as_i64()),
            needs_duration_resolve,
            is_cover,
        })
    }
}

use catalog_normalize::normalize_title;

use crate::sc_payload::{parse_dt, string_field};

fn extract_isrc(payload: &Value) -> Option<String> {
    let candidates = [
        payload.pointer("/publisher_metadata/isrc"),
        payload.pointer("/publisher_metadata/iswc"),
        payload.get("isrc"),
    ];
    for c in candidates.into_iter().flatten() {
        if let Some(s) = c.as_str() {
            let s = s.trim();
            if is_valid_isrc(s) {
                return Some(s.to_uppercase());
            }
        }
    }
    None
}

fn is_valid_isrc(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}
