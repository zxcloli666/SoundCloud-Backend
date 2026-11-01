use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::Kind;

#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum MatchScAccount {
    Upsert {
        artist_id: Uuid,
        sc_user_id: String,
        role: String,
        source: String,
        verified: bool,
        at: DateTime<Utc>,
    },
    Delete {
        artist_id: Uuid,
        sc_user_id: String,
    },
}

impl Kind for MatchScAccount {
    const NAME: &'static str = "match.sc_account";
    type Output = bool;
}

#[derive(Debug, Serialize)]
pub struct MatchWantedLink {
    pub wanted_id: Uuid,
    pub sc_track_id: String,
    pub at: DateTime<Utc>,
}

impl Kind for MatchWantedLink {
    const NAME: &'static str = "match.wanted_link";
    type Output = Option<Uuid>;
}

#[derive(Debug, Serialize)]
pub struct MatchGeniusSong {
    pub track_id: Uuid,
    pub genius_song_id: i64,
}

impl Kind for MatchGeniusSong {
    const NAME: &'static str = "match.genius_song";
    type Output = Option<String>;
}

#[derive(Debug, Serialize)]
pub struct AlbumSlot {
    pub album_id: Uuid,
    pub position: i16,
}

#[derive(Debug, Serialize)]
pub struct MatchAlbumTrack {
    pub track_id: Uuid,
    pub albums: Vec<AlbumSlot>,
}

impl Kind for MatchAlbumTrack {
    const NAME: &'static str = "match.album_track";
    type Output = ();
}
