use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistSnapshot {
    pub playlist_id: String,
    pub owner_id: String,
    pub track_ids: Vec<String>,
    pub hydrated_tracks: Vec<HydratedTrack>,
    pub track_count: i32,
    pub remote_last_modified: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HydratedTrack {
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
    pub sc_created_at: Option<DateTime<Utc>>,
    pub sc_last_modified: Option<DateTime<Utc>>,
    pub uploader_sc_user_id: Option<String>,
    pub uploader_urn: Option<String>,
    pub uploader_username: Option<String>,
    pub uploader_avatar_url: Option<String>,
    pub play_count_sc: Option<i64>,
    pub likes_count_sc: Option<i64>,
    pub reposts_count_sc: Option<i64>,
    pub comments_count_sc: Option<i64>,
    pub needs_duration_resolve: bool,
}
