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
