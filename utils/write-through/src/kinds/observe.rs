use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::Kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Upsert,
    Absent,
    Bump,
}

#[derive(Debug, Serialize)]
pub struct ObserveTracks {
    pub mode: Mode,
    pub observed_at: DateTime<Utc>,
    pub observation: i64,
    pub rows: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct ObservedTrack {
    pub sc_track_id: String,
    pub id: Uuid,
    pub was_new: bool,
    pub applied: bool,
}

impl Kind for ObserveTracks {
    const NAME: &'static str = "observe.tracks";
    type Output = Vec<ObservedTrack>;
}

#[derive(Debug, Serialize)]
pub struct ObserveUsers {
    pub mode: Mode,
    pub observed_at: DateTime<Utc>,
    pub observation: i64,
    pub rows: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct ObservedUser {
    pub sc_user_id: String,
    pub was_new: bool,
}

impl Kind for ObserveUsers {
    const NAME: &'static str = "observe.users";
    type Output = Vec<ObservedUser>;
}

#[derive(Debug, Serialize)]
pub struct ObservePlaylists {
    pub mode: Mode,
    pub observed_at: DateTime<Utc>,
    pub observation: i64,
    pub rows: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct ObservedPlaylist {
    pub urn: String,
    pub was_new: bool,
}

impl Kind for ObservePlaylists {
    const NAME: &'static str = "observe.playlists";
    type Output = Vec<ObservedPlaylist>;
}

#[derive(Debug, Serialize)]
pub struct ObserveProfile {
    pub user_id: String,
    pub profile: Value,
    pub observed_at: DateTime<Utc>,
    pub observation: i64,
}

impl Kind for ObserveProfile {
    const NAME: &'static str = "observe.profile";
    type Output = ();
}
