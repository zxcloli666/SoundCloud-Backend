use serde::{Deserialize, Serialize};

pub const LYRICS_SEARCH_SUBJECT: &str = "lyrics.search.v1";
pub const LYRICS_SEARCH_QUEUE: &str = "jobs-lyrics-search-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsSearchRequest {
    pub query: String,
    pub duration_sec: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsSearchReply {
    pub reachable: bool,
    pub candidates: Vec<LyricsSearchCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsSearchCandidate {
    pub lrclib_id: i64,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub duration_sec: Option<i64>,
    pub synced_lrc: Option<String>,
    pub plain_text: Option<String>,
}
