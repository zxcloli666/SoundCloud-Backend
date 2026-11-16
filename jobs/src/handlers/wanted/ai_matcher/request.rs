use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MatchTrackRequest {
    pub target: MatchTarget,
    pub candidates: Vec<MatchCandidate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MatchTarget {
    pub artist: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MatchCandidate {
    pub id: u32,
    pub artist: String,
    pub title: String,
    pub duration_sec: Option<f64>,
}
