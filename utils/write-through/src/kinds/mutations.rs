use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::Kind;

#[derive(Debug, Serialize)]
pub struct TrackEdit {
    pub sc_track_id: String,
    pub uploader: String,
    pub desired: Value,
    pub observation: i64,
    pub at: DateTime<Utc>,
}

impl Kind for TrackEdit {
    const NAME: &'static str = "track.edit";
    type Output = Option<String>;
}

#[derive(Debug, Serialize)]
pub struct TrackDelete {
    pub sc_track_id: String,
    pub uploader: String,
    pub observation: i64,
    pub at: DateTime<Utc>,
}

impl Kind for TrackDelete {
    const NAME: &'static str = "track.delete";
    type Output = Option<String>;
}

#[derive(Debug, Serialize)]
pub struct PlaylistEdit {
    pub urn: String,
    pub owner: String,
    pub desired: Value,
    pub observation: i64,
    pub at: DateTime<Utc>,
}

impl Kind for PlaylistEdit {
    const NAME: &'static str = "playlist.edit";
    type Output = Option<String>;
}

#[derive(Debug, Serialize)]
pub struct PlaylistDelete {
    pub urn: String,
    pub owner: String,
    pub observation: i64,
    pub at: DateTime<Utc>,
}

impl Kind for PlaylistDelete {
    const NAME: &'static str = "playlist.delete";
    type Output = Option<String>;
}

#[derive(Debug, Serialize)]
pub struct OwnedInsert {
    pub user_id: String,
    pub sc_track_id: String,
    pub variants: Vec<String>,
    pub at: DateTime<Utc>,
}

impl Kind for OwnedInsert {
    const NAME: &'static str = "owned.insert";
    type Output = bool;
}

#[derive(Debug, Serialize)]
#[serde(tag = "entity", rename_all = "snake_case")]
pub enum OwnedDelete {
    Track {
        variants: Vec<String>,
        sc_track_id: String,
    },
    Playlist {
        variants: Vec<String>,
        playlist_urn: String,
    },
}

impl Kind for OwnedDelete {
    const NAME: &'static str = "owned.delete";
    type Output = usize;
}

#[derive(Debug, Serialize)]
pub struct CommentPost {
    pub id: Uuid,
    pub sc_track_id: String,
    pub user_urn: String,
    pub body: String,
    pub track_position_ms: Option<i64>,
    pub at: DateTime<Utc>,
}

impl Kind for CommentPost {
    const NAME: &'static str = "comment.post";
    type Output = bool;
}

#[derive(Debug, Serialize)]
pub struct MembershipOp {
    pub playlist_urn: String,
    pub actor: String,
    pub variants: Vec<String>,
    pub expected_projection_revision: Option<i64>,
    pub operations: Vec<MembershipOperation>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct MembershipOperation {
    pub operation_id: Uuid,
    pub idempotency_key: Uuid,
    pub fingerprint: String,
    pub kind: String,
    pub track_id: Option<String>,
    pub left_anchor_track_id: Option<String>,
    pub right_anchor_track_id: Option<String>,
    pub boundary: Option<String>,
    pub ordered_track_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct MembershipOutcome {
    pub status: String,
    pub code: Option<String>,
    pub appended: i64,
    pub projection_revision: i64,
    pub added: Vec<String>,
}

impl Kind for MembershipOp {
    const NAME: &'static str = "playlist.membership_op";
    type Output = MembershipOutcome;
}
