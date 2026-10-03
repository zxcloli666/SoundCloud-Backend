use std::time::Duration;

use axum::http::HeaderMap;
use catalog_normalize::normalize_name;
use sc_transport::SearchType;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::common::admission::Endpoint;
use crate::config::LiveMode;
use crate::modules::search::query::{PlaylistSearchQuery, TrackSearchQuery};

pub const INTENT_HEADER: &str = "x-search-intent";
pub const LOCAL_PATIENCE: Duration = Duration::from_millis(600);
pub const LOCAL_CAP: Duration = Duration::from_millis(3000);
pub const PROXY_MIN_LEFT: Duration = Duration::from_millis(800);
pub const ATTACH_CAP: Duration = Duration::from_millis(500);
pub const ADOPT_CAP: Duration = Duration::from_millis(1000);
pub const IMPORT_PACING: Duration = Duration::from_secs(15);
pub const WINDOW_FRESH_SECONDS: i64 = 600;
pub const WINDOW_TTL_OK: u64 = 1800;
pub const WINDOW_TTL_EMPTY: u64 = 900;
pub const WINDOW_TTL_IMPORT: u64 = 600;
pub const ITEM_TTL: u64 = 1800;
pub const FAIL_TTL: u64 = 60;
pub const TRACKS_ENOUGH_ROWS: usize = 10;
pub const SIDE_ENOUGH_ROWS: usize = 5;
pub const IMPORT_LOCAL_ROWS: i64 = 20;

const MAX_QUERY_CHARS: usize = 128;
const MIN_NORMALIZED_CHARS: usize = 3;
const IMPORT_MAX_LIMIT: i64 = 5;
const RESCUE_MIN_TOKENS: usize = 2;
const RESCUE_MIN_CHARS: usize = 5;
const LINK_MARKERS: [&str; 3] = ["soundcloud.com", "snd.sc", "://"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveKind {
    Tracks,
    Users,
    Playlists,
}

impl LiveKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tracks => "tracks",
            Self::Users => "users",
            Self::Playlists => "playlists",
        }
    }

    pub fn search_type(self) -> SearchType {
        match self {
            Self::Tracks => SearchType::Tracks,
            Self::Users => SearchType::Users,
            Self::Playlists => SearchType::PlaylistsWithoutAlbums,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveClass {
    Main,
    Side,
    Fill,
    Import,
    Match,
    Rescue,
}

impl LiveClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Side => "side",
            Self::Fill => "fill",
            Self::Import => "import",
            Self::Match => "match",
            Self::Rescue => "rescue",
        }
    }

    pub fn endpoint(self) -> Endpoint {
        match self {
            Self::Main | Self::Fill => Endpoint::LiveMain,
            Self::Side => Endpoint::LiveSide,
            Self::Import | Self::Match => Endpoint::LiveImport,
            Self::Rescue => Endpoint::LiveRescue,
        }
    }

    pub fn proxy_allowed(self) -> bool {
        matches!(self, Self::Main | Self::Side | Self::Import | Self::Match)
    }

    pub fn budget(self) -> Duration {
        match self {
            Self::Main | Self::Side => Duration::from_millis(3000),
            Self::Fill | Self::Rescue => Duration::from_millis(2500),
            Self::Import | Self::Match => Duration::from_millis(4500),
        }
    }

    pub fn relay_wait(self) -> Duration {
        match self {
            Self::Import | Self::Match => Duration::from_millis(3000),
            _ => Duration::from_millis(2000),
        }
    }

    pub fn paces(self) -> bool {
        self == Self::Import
    }

    pub fn allowed_in(self, mode: LiveMode, rescue: bool) -> bool {
        match (mode, self) {
            (LiveMode::Off, _) => false,
            (_, Self::Main | Self::Side | Self::Import | Self::Match) => true,
            (LiveMode::Auto, Self::Fill) => true,
            (LiveMode::Auto, Self::Rescue) => rescue,
            (LiveMode::Explicit, Self::Fill | Self::Rescue) => false,
        }
    }

    pub fn scope(self, kind: LiveKind) -> &'static str {
        match self {
            Self::Import | Self::Match => "import",
            _ => kind.as_str(),
        }
    }

    pub fn window_size(self, kind: LiveKind) -> i64 {
        match (self, kind) {
            (Self::Import | Self::Match, _) => 10,
            (_, LiveKind::Tracks) => 40,
            (_, LiveKind::Users | LiveKind::Playlists) => 20,
        }
    }

    pub fn window_ttl(self, empty: bool) -> u64 {
        match (self, empty) {
            (Self::Import | Self::Match, _) => WINDOW_TTL_IMPORT,
            (_, true) => WINDOW_TTL_EMPTY,
            (_, false) => WINDOW_TTL_OK,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Absent,
    Fill,
    Import,
    Other,
}

impl Intent {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        match headers.get(INTENT_HEADER).map(|value| value.to_str()) {
            None => Self::Absent,
            Some(Ok(value)) if value.trim().eq_ignore_ascii_case("fill") => Self::Fill,
            Some(Ok(value)) if value.trim().eq_ignore_ascii_case("import") => Self::Import,
            Some(_) => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveQuery {
    pub text: String,
    pub norm: String,
    pub hash: String,
}

impl LiveQuery {
    pub fn parse(raw: &str) -> Option<Self> {
        let cleaned: String = raw.trim().chars().filter(|c| !c.is_control()).collect();
        let text: String = cleaned.trim().chars().take(MAX_QUERY_CHARS).collect();
        let text = text.trim().to_owned();
        let lowered = text.to_lowercase();
        if LINK_MARKERS.iter().any(|marker| lowered.contains(marker)) {
            return None;
        }
        let norm = normalize_name(&text);
        if norm.chars().count() < MIN_NORMALIZED_CHARS {
            return None;
        }
        let hash = hex::encode(Sha256::digest(norm.as_bytes()))[..32].to_owned();
        Some(Self { text, norm, hash })
    }

    pub fn short_hash(&self) -> &str {
        &self.hash[..8]
    }

    pub fn is_specific(&self) -> bool {
        self.norm.split_whitespace().count() >= RESCUE_MIN_TOKENS
            || self.norm.chars().count() >= RESCUE_MIN_CHARS
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct LiveParams {
    #[serde(default)]
    pub linked_partitioning: Option<String>,
}

impl LiveParams {
    pub fn linked(&self) -> bool {
        self.linked_partitioning
            .as_deref()
            .is_some_and(|value| matches!(value, "true" | "1"))
    }
}

pub fn plain_tracks(query: &TrackSearchQuery) -> Option<&str> {
    let filtered = query.ids.is_some()
        || query.genres.is_some()
        || query.tags.is_some()
        || query.user_urn.is_some()
        || query.access.is_some();
    query.q.as_deref().filter(|_| !filtered)
}

pub fn plain_playlists(query: &PlaylistSearchQuery) -> Option<&str> {
    let filtered = query.user_urn.is_some()
        || query.access.is_some()
        || query
            .show_tracks
            .as_deref()
            .is_some_and(|value| !matches!(value, "false" | "0"));
    query.q.as_deref().filter(|_| !filtered)
}

pub fn class_of(
    kind: LiveKind,
    intent: Intent,
    limit: i64,
    page_given: bool,
    linked_partitioning: bool,
) -> LiveClass {
    match kind {
        LiveKind::Users | LiveKind::Playlists => LiveClass::Side,
        LiveKind::Tracks if intent == Intent::Import => LiveClass::Import,
        LiveKind::Tracks if linked_partitioning && limit <= IMPORT_MAX_LIMIT && !page_given => {
            LiveClass::Import
        }
        LiveKind::Tracks if intent == Intent::Fill => LiveClass::Fill,
        LiveKind::Tracks => LiveClass::Main,
    }
}

pub fn identity_of(sc_user_id: &str) -> String {
    hex::encode(Sha256::digest(sc_user_id.as_bytes()))[..16].to_owned()
}
