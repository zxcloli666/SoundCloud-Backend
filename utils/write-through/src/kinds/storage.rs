use chrono::{DateTime, Utc};
use serde::Serialize;

use super::Kind;

#[derive(Debug, Serialize)]
pub struct TrackStorageLost {
    pub urn: String,
    pub at: DateTime<Utc>,
}

impl Kind for TrackStorageLost {
    const NAME: &'static str = "track.storage_lost";
    type Output = bool;
}

#[derive(Debug, Serialize)]
pub struct TrackTooLong {
    pub sc_track_id: String,
    pub at: DateTime<Utc>,
}

impl Kind for TrackTooLong {
    const NAME: &'static str = "track.too_long";
    type Output = bool;
}

#[derive(Debug, Serialize)]
pub struct TrackReleaseHeld {
    pub sc_track_id: String,
    pub at: DateTime<Utc>,
}

impl Kind for TrackReleaseHeld {
    const NAME: &'static str = "track.release_held";
    type Output = Option<i32>;
}

#[derive(Debug, Serialize)]
pub struct TrackS3State {
    pub ok: Vec<String>,
    pub missing: Vec<String>,
    pub at: DateTime<Utc>,
}

impl Kind for TrackS3State {
    const NAME: &'static str = "track.s3_state";
    type Output = Option<i64>;
}
