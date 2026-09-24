use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use catalog_normalize::{name_similarity, normalize_title, title_forms, works_match};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use wreq::header::{ACCEPT, COOKIE, HeaderMap, HeaderValue, USER_AGENT};

use crate::{ExternalFetcher, GeniusService, SourceError, SourceResult};

const LOOKUP_METHOD: &str = call_lua_macros::lua_script!("lyrics_methods/lookup.lua");
const LOOKUP_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const MAX_CANDIDATES: usize = 24;
const MAX_QUERIES: usize = 4;
const MAX_LYRICS_BYTES: usize = 800 * 1024;
const LRCLIB_API: &str = "https://lrclib.net/api";
const USER_AGENT_VALUE: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";
const MUSIXMATCH_APP_ID: &str = "web-desktop-app-v1.0";
const MUSIXMATCH_TOKEN_TTL: Duration = Duration::from_secs(9 * 60 * 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LyricsHints {
    pub title: String,
    pub artist: String,
    pub duration_sec: Option<i64>,
    pub genius_song_id: Option<i64>,
    pub genius_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LyricsCandidate {
    pub source: String,
    pub synced_lrc: Option<String>,
    pub plain_text: Option<String>,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub duration_sec: Option<i64>,
    pub exact: bool,
    pub query_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LyricsFailure {
    pub class: String,
    pub retry_after_seconds: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LyricsLookupOutcome {
    Found(LyricsCandidate),
    CleanMiss,
    Retry(LyricsFailure),
    InsufficientMetadata,
}

#[derive(Clone)]
pub struct LyricsSources {
    fetcher: Arc<ExternalFetcher>,
    genius: Arc<GeniusService>,
    musixmatch: Arc<MusixmatchLyrics>,
}

#[derive(Serialize)]
struct LuaInputs<'a> {
    artist: &'a str,
    title: &'a str,
    duration_sec: Option<i64>,
    queries: &'a [String],
    genius_song_id: Option<i64>,
    genius_url: Option<&'a str>,
    musixmatch_base: &'a str,
}

#[derive(Deserialize)]
struct LuaEnvelope {
    kind: String,
    #[serde(default)]
    candidates: Vec<LuaCandidate>,
}

#[derive(Deserialize)]
struct LuaCandidate {
    source: String,
    #[serde(default)]
    synced_lrc: Option<String>,
    #[serde(default)]
    plain_text: Option<String>,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration_sec: Option<f64>,
    #[serde(default)]
    exact: bool,
    #[serde(default)]
    query_index: usize,
}

#[derive(Debug, Deserialize)]
struct LrclibRaw {
    #[serde(default, rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(default, rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(default, rename = "artistName")]
    artist_name: Option<String>,
    #[serde(default, rename = "trackName")]
    track_name: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

impl LyricsSources {
    pub fn new(
        fetcher: Arc<ExternalFetcher>,
        genius: Arc<GeniusService>,
        musixmatch_base: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            musixmatch: Arc::new(MusixmatchLyrics::new(fetcher.clone(), musixmatch_base)),
            fetcher,
            genius,
        })
    }

    pub async fn lookup(&self, hints: &LyricsHints) -> LyricsLookupOutcome {
        let title = hints.title.trim();
        if title.is_empty() {
            return LyricsLookupOutcome::InsufficientMetadata;
        }
        let artist = hints.artist.trim();
        let queries = heuristic_queries(artist, title);
        match self.lookup_lua(hints, &queries).await {
            Ok(outcome) => outcome,
            Err(lua_error) => self.lookup_raw(hints, &queries, lua_error).await,
        }
    }

    async fn lookup_lua(
        &self,
        hints: &LyricsHints,
        queries: &[String],
    ) -> SourceResult<LyricsLookupOutcome> {
        let payload = serde_json::to_vec(&LuaInputs {
            artist: hints.artist.trim(),
            title: hints.title.trim(),
            duration_sec: hints.duration_sec,
            queries,
            genius_song_id: hints.genius_song_id,
            genius_url: hints.genius_url.as_deref(),
            musixmatch_base: self.musixmatch.base.as_str(),
        })
        .map_err(|error| SourceError::Invalid(format!("lyrics lua input: {error}")))?;
        let output = self
            .fetcher
            .call_method(
                "lyrics.lookup",
                LOOKUP_METHOD,
                Bytes::from(payload),
                LOOKUP_OUTPUT_LIMIT,
            )
            .await?;
        let envelope: LuaEnvelope = serde_json::from_slice(&output)
            .map_err(|error| SourceError::Invalid(format!("lyrics lua output: {error}")))?;
        match envelope.kind.as_str() {
            "empty" if envelope.candidates.is_empty() => Ok(LyricsLookupOutcome::CleanMiss),
            "found" => {
                let candidates = envelope
                    .candidates
                    .into_iter()
                    .take(MAX_CANDIDATES)
                    .filter_map(lua_candidate)
                    .collect();
                Ok(select_candidate(hints, candidates)
                    .map(LyricsLookupOutcome::Found)
                    .unwrap_or(LyricsLookupOutcome::CleanMiss))
            }
            _ => Err(SourceError::Invalid(
                "lyrics lua returned an unknown envelope".to_owned(),
            )),
        }
    }

    async fn lookup_raw(
        &self,
        hints: &LyricsHints,
        queries: &[String],
        lua_error: SourceError,
    ) -> LyricsLookupOutcome {
        let mut failures = vec![failure(&lua_error)];
        if let Some(url) = hints
            .genius_url
            .as_deref()
            .filter(|url| !url.trim().is_empty())
        {
            match validate_genius_url(url).map(str::to_owned) {
                Ok(url) => match self.genius.lyrics_by_url_strict(&url).await {
                    Ok(Some(plain_text)) => {
                        if let Some(candidate) = normalize_candidate(LyricsCandidate {
                            source: "genius".to_owned(),
                            synced_lrc: None,
                            plain_text: Some(plain_text),
                            artist: None,
                            title: None,
                            duration_sec: None,
                            exact: true,
                            query_index: 0,
                        }) {
                            return LyricsLookupOutcome::Found(candidate);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => failures.push(failure(&error)),
                },
                Err(error) => failures.push(failure(&error)),
            }
        }
        if let Some(genius_song_id) = hints.genius_song_id {
            match self.genius.lyrics_by_song_id_strict(genius_song_id).await {
                Ok(Some(plain_text)) => {
                    if let Some(candidate) = normalize_candidate(LyricsCandidate {
                        source: "genius".to_owned(),
                        synced_lrc: None,
                        plain_text: Some(plain_text),
                        artist: None,
                        title: None,
                        duration_sec: None,
                        exact: true,
                        query_index: 0,
                    }) {
                        return LyricsLookupOutcome::Found(candidate);
                    }
                }
                Ok(None) => {}
                Err(error) => failures.push(failure(&error)),
            }
        }

        let mut candidates = Vec::new();
        let mut source_failures = Vec::new();
        collect_source(
            self.musixmatch.search(hints, 0).await,
            &mut candidates,
            &mut source_failures,
        );
        for (query_index, query) in queries.iter().take(MAX_QUERIES).enumerate() {
            let (lrclib, genius) = tokio::join!(
                self.search_lrclib(query, query_index + 1),
                self.search_genius(query, query_index + 1),
            );
            collect_source(lrclib, &mut candidates, &mut source_failures);
            collect_source(genius, &mut candidates, &mut source_failures);
        }
        if let Some(candidate) = select_candidate(hints, candidates) {
            return LyricsLookupOutcome::Found(candidate);
        }
        failures.extend(source_failures);
        if failures.len() > 1 {
            return LyricsLookupOutcome::Retry(merge_failures(failures));
        }
        LyricsLookupOutcome::CleanMiss
    }

    async fn search_lrclib(
        &self,
        query: &str,
        query_index: usize,
    ) -> SourceResult<Vec<LyricsCandidate>> {
        let url = format!("{LRCLIB_API}/search?q={}", urlencoding::encode(query));
        let bytes = self.fetcher.get_bytes(&url, json_headers()).await?;
        let entries: Vec<LrclibRaw> = serde_json::from_slice(&bytes)
            .map_err(|error| SourceError::Invalid(format!("lrclib search: {error}")))?;
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                normalize_candidate(LyricsCandidate {
                    source: "lrclib".to_owned(),
                    synced_lrc: entry.synced_lyrics,
                    plain_text: entry.plain_lyrics,
                    artist: entry.artist_name,
                    title: entry.track_name,
                    duration_sec: entry.duration.map(|duration| duration.round() as i64),
                    exact: false,
                    query_index,
                })
            })
            .take(10)
            .collect())
    }

    async fn search_genius(
        &self,
        query: &str,
        query_index: usize,
    ) -> SourceResult<Vec<LyricsCandidate>> {
        Ok(self
            .genius
            .search_by_query_strict(query, 3)
            .await?
            .into_iter()
            .filter_map(|entry| {
                normalize_candidate(LyricsCandidate {
                    source: "genius".to_owned(),
                    synced_lrc: None,
                    plain_text: Some(entry.plain_text),
                    artist: entry.artist_guess,
                    title: entry.title_guess,
                    duration_sec: None,
                    exact: false,
                    query_index,
                })
            })
            .collect())
    }
}

fn collect_source(
    result: SourceResult<Vec<LyricsCandidate>>,
    candidates: &mut Vec<LyricsCandidate>,
    failures: &mut Vec<LyricsFailure>,
) {
    match result {
        Ok(mut source_candidates) => candidates.append(&mut source_candidates),
        Err(error) => failures.push(failure(&error)),
    }
}

fn lua_candidate(candidate: LuaCandidate) -> Option<LyricsCandidate> {
    normalize_candidate(LyricsCandidate {
        source: candidate.source,
        synced_lrc: candidate.synced_lrc,
        plain_text: candidate.plain_text,
        artist: candidate.artist,
        title: candidate.title,
        duration_sec: candidate
            .duration_sec
            .map(|duration| duration.round() as i64),
        exact: candidate.exact,
        query_index: candidate.query_index,
    })
}

fn normalize_candidate(mut candidate: LyricsCandidate) -> Option<LyricsCandidate> {
    if !matches!(
        candidate.source.as_str(),
        "lrclib" | "musixmatch" | "genius" | "netease"
    ) {
        return None;
    }
    candidate.synced_lrc = normalize_synced(candidate.synced_lrc);
    candidate.plain_text = normalize_plain(candidate.plain_text);
    if candidate.plain_text.is_none()
        && let Some(synced_lrc) = candidate.synced_lrc.as_deref()
    {
        candidate.plain_text = normalize_plain(Some(strip_lrc_timestamps(synced_lrc)));
    }
    if candidate.synced_lrc.is_none() && candidate.plain_text.is_none() {
        return None;
    }
    candidate.artist = nonblank(candidate.artist);
    candidate.title = nonblank(candidate.title);
    Some(candidate)
}

fn clean_musixmatch_plain(value: String) -> Option<String> {
    let cleaned = value
        .lines()
        .take_while(|line| {
            let lowercase = line.to_lowercase();
            !lowercase.contains("this lyrics is not for commercial use") && !line.contains("*****")
        })
        .collect::<Vec<_>>()
        .join("\n");
    normalize_plain(Some(cleaned))
}

fn normalize_plain(value: Option<String>) -> Option<String> {
    let value = value?;
    if value.contains('\0') || value.len() > MAX_LYRICS_BYTES {
        return None;
    }
    let value = value.trim();
    (value.chars().count() > 20).then(|| value.to_owned())
}

fn normalize_synced(value: Option<String>) -> Option<String> {
    let value = value?;
    if value.contains('\0') || value.len() > MAX_LYRICS_BYTES || !contains_lrc_timestamp(&value) {
        return None;
    }
    Some(value.trim().to_owned())
}

fn nonblank(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn select_candidate(
    hints: &LyricsHints,
    candidates: Vec<LyricsCandidate>,
) -> Option<LyricsCandidate> {
    let mut by_body: HashMap<String, LyricsCandidate> = HashMap::new();
    for candidate in candidates
        .into_iter()
        .filter(|candidate| matches_hints(hints, candidate))
    {
        let body = candidate
            .plain_text
            .as_deref()
            .or(candidate.synced_lrc.as_deref())?;
        let key = normalize_body(body);
        if key.is_empty() {
            continue;
        }
        match by_body.get(&key) {
            Some(existing) if candidate_quality(existing) >= candidate_quality(&candidate) => {}
            _ => {
                by_body.insert(key, candidate);
            }
        }
    }
    let mut candidates: Vec<LyricsCandidate> = by_body.into_values().collect();
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate_rank(hints, candidate)));
    candidates.into_iter().next()
}

fn matches_hints(hints: &LyricsHints, candidate: &LyricsCandidate) -> bool {
    if candidate.exact {
        return true;
    }
    let Some(title) = candidate.title.as_deref() else {
        return false;
    };
    let target = title_forms(&hints.title);
    let other = title_forms(title);
    let title_matches = target.is_same_recording(&other)
        || works_match(&target, &other)
        || normalize_title(&hints.title) == normalize_title(title);
    if !title_matches {
        return false;
    }
    if let Some(artist) = candidate.artist.as_deref()
        && !hints.artist.trim().is_empty()
        && name_similarity(&hints.artist, artist) < 0.72
    {
        return false;
    }
    if let (Some(target), Some(candidate_duration)) = (hints.duration_sec, candidate.duration_sec)
        && target > 0
        && candidate_duration > 0
    {
        let maximum = target.max(candidate_duration) as f64;
        let difference = (target - candidate_duration).abs() as f64 / maximum;
        if difference > 0.25 {
            return false;
        }
    }
    true
}

fn candidate_rank(
    hints: &LyricsHints,
    candidate: &LyricsCandidate,
) -> (u8, u8, u16, u16, u8, usize) {
    let title_score = candidate
        .title
        .as_deref()
        .map(|title| (name_similarity(&hints.title, title) * 1000.0) as u16)
        .unwrap_or(0);
    let artist_score = candidate
        .artist
        .as_deref()
        .map(|artist| (name_similarity(&hints.artist, artist) * 1000.0) as u16)
        .unwrap_or(0);
    (
        u8::from(candidate.exact),
        u8::from(candidate.synced_lrc.is_some()),
        title_score,
        artist_score,
        source_rank(&candidate.source),
        usize::MAX.saturating_sub(candidate.query_index),
    )
}

fn candidate_quality(candidate: &LyricsCandidate) -> (u8, usize, u8) {
    (
        u8::from(candidate.synced_lrc.is_some()),
        candidate
            .plain_text
            .as_deref()
            .map(str::len)
            .unwrap_or_default(),
        source_rank(&candidate.source),
    )
}

fn source_rank(source: &str) -> u8 {
    match source {
        "lrclib" => 4,
        "genius" => 3,
        "musixmatch" => 2,
        "netease" => 1,
        _ => 0,
    }
}

fn normalize_body(value: &str) -> String {
    let value = strip_lrc_timestamps(value).to_lowercase();
    value
        .chars()
        .filter(|character| character.is_alphanumeric() || character.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn strip_lrc_timestamps(value: &str) -> String {
    value
        .lines()
        .filter(|line| !is_lrc_metadata(line))
        .map(strip_lrc_line)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains_lrc_timestamp(value: &str) -> bool {
    value.lines().any(|line| {
        line.match_indices('[')
            .any(|(start, _)| lrc_timestamp_end(line, start).is_some())
    })
}

fn strip_lrc_line(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut cursor = 0;
    while let Some(relative) = line[cursor..].find('[') {
        let start = cursor + relative;
        output.push_str(&line[cursor..start]);
        match lrc_timestamp_end(line, start) {
            Some(end) => cursor = end,
            None => {
                output.push('[');
                cursor = start + 1;
            }
        }
    }
    output.push_str(&line[cursor..]);
    output.trim().to_owned()
}

fn lrc_timestamp_end(value: &str, start: usize) -> Option<usize> {
    let tail = value.get(start + 1..)?;
    let closing = tail.find(']')?;
    if closing > 10 {
        return None;
    }
    let inner = &tail[..closing];
    let (minutes, seconds) = inner.split_once(':')?;
    if minutes.is_empty() || minutes.len() > 3 || !minutes.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let (whole, fraction) = seconds
        .split_once('.')
        .map_or((seconds, None), |(whole, fraction)| (whole, Some(fraction)));
    if whole.len() != 2 || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if fraction.is_some_and(|fraction| {
        fraction.is_empty()
            || fraction.len() > 3
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return None;
    }
    Some(start + closing + 2)
}

fn is_lrc_metadata(line: &str) -> bool {
    let line = line.trim();
    let Some(inner) = line
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return false;
    };
    let Some((key, _)) = inner.split_once(':') else {
        return false;
    };
    matches!(
        key.to_ascii_lowercase().as_str(),
        "ar" | "al" | "ti" | "au" | "by" | "offset" | "length" | "re"
    )
}

fn normalize_query(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn heuristic_queries(artist: &str, title: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut queries = Vec::new();
    for value in [format!("{artist} {title}"), title.to_owned()] {
        let value = normalize_query(&value);
        let key = value.to_lowercase();
        if value.chars().count() >= 2 && seen.insert(key) {
            queries.push(value);
        }
    }
    if let Some((embedded_artist, embedded_title)) = split_artist_title(title) {
        for value in [
            format!("{embedded_artist} {embedded_title}"),
            embedded_title,
        ] {
            let value = normalize_query(&value);
            let key = value.to_lowercase();
            if value.chars().count() >= 2 && seen.insert(key) {
                queries.push(value);
            }
        }
    }
    queries.into_iter().take(MAX_QUERIES).collect()
}

fn split_artist_title(value: &str) -> Option<(String, String)> {
    for separator in [" - ", " – ", " — ", " // "] {
        let Some(index) = value.find(separator) else {
            continue;
        };
        let artist = value[..index].trim();
        let title = value[index + separator.len()..].trim();
        if !artist.is_empty() && !title.is_empty() {
            return Some((artist.to_owned(), title.to_owned()));
        }
    }
    None
}

fn failure(error: &SourceError) -> LyricsFailure {
    let class = match error {
        SourceError::Status { status: 429, .. } => "rate_limited",
        SourceError::Status {
            status: 401 | 403, ..
        } => "blocked",
        SourceError::Status { status, .. } if *status >= 500 => "upstream",
        SourceError::Status { .. } => "protocol",
        SourceError::Unreachable(_) => "transport",
        SourceError::Invalid(_) => "malformed",
        SourceError::Oversized { .. } => "oversized",
        SourceError::NotConfigured(_) => "not_configured",
    };
    LyricsFailure {
        class: class.to_owned(),
        retry_after_seconds: error.retry_after_seconds(),
    }
}

fn merge_failures(failures: Vec<LyricsFailure>) -> LyricsFailure {
    let retry_after_seconds = failures
        .iter()
        .filter_map(|failure| failure.retry_after_seconds)
        .max();
    let class = failures
        .into_iter()
        .map(|failure| failure.class)
        .find(|class| class != "not_configured")
        .unwrap_or_else(|| "not_configured".to_owned());
    LyricsFailure {
        class,
        retry_after_seconds,
    }
}

fn validate_genius_url(value: &str) -> SourceResult<&str> {
    let url = url::Url::parse(value)
        .map_err(|error| SourceError::Invalid(format!("genius url: {error}")))?;
    let valid = url.scheme() == "https"
        && matches!(url.host_str(), Some("genius.com" | "www.genius.com"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(SourceError::Invalid("genius url is not allowed".to_owned()));
    }
    Ok(value)
}

fn json_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

fn musixmatch_headers(guid: &str) -> SourceResult<HeaderMap> {
    let mut headers = json_headers();
    let cookie = HeaderValue::from_str(&format!("x-mxm-token-guid={guid}"))
        .map_err(|error| SourceError::Invalid(format!("musixmatch cookie: {error}")))?;
    headers.insert(COOKIE, cookie);
    Ok(headers)
}

struct TokenCache {
    token: String,
    expires_at: Instant,
}

struct MusixmatchLyrics {
    fetcher: Arc<ExternalFetcher>,
    base: String,
    guid: String,
    token: Mutex<Option<TokenCache>>,
}

#[derive(Deserialize)]
struct MxmTokenBody {
    user_token: Option<String>,
}

#[derive(Deserialize)]
struct MxmTrack {
    track_name: Option<String>,
    artist_name: Option<String>,
    track_length: Option<i64>,
    instrumental: Option<i64>,
}

#[derive(Deserialize)]
struct MxmRichLine {
    ts: Option<f64>,
    x: Option<String>,
    #[serde(default)]
    l: Vec<MxmRichWord>,
}

#[derive(Deserialize)]
struct MxmRichWord {
    c: Option<String>,
}

impl MusixmatchLyrics {
    fn new(fetcher: Arc<ExternalFetcher>, base: String) -> Self {
        Self {
            fetcher,
            base: base.trim_end_matches('/').to_owned(),
            guid: uuid::Uuid::new_v4().to_string(),
            token: Mutex::new(None),
        }
    }

    async fn search(
        &self,
        hints: &LyricsHints,
        query_index: usize,
    ) -> SourceResult<Vec<LyricsCandidate>> {
        let mut token = self.token().await?;
        let mut payload = self.macro_payload(hints, &token).await?;
        if musixmatch_status(&payload) == Some(401) {
            self.clear_token().await;
            token = self.token().await?;
            payload = self.macro_payload(hints, &token).await?;
        }
        let status = musixmatch_status(&payload).unwrap_or_default();
        if status != 200 {
            return Err(SourceError::Status {
                status: u16::try_from(status).unwrap_or(500),
                body: "musixmatch macro request failed".to_owned(),
                retry_after_seconds: None,
            });
        }
        let root = payload
            .pointer("/message/body/macro_calls")
            .unwrap_or(&payload);
        let track = deep_find(root, "track")
            .cloned()
            .and_then(|track| serde_json::from_value::<MxmTrack>(track).ok());
        if track
            .as_ref()
            .and_then(|track| track.instrumental)
            .unwrap_or_default()
            > 0
        {
            return Ok(Vec::new());
        }
        let richsync = deep_find_string(root, "richsync_body").and_then(richsync_to_lrc);
        let subtitle = deep_find_string(root, "subtitle_body")
            .and_then(|subtitle| normalize_synced(Some(subtitle)));
        let synced_lrc = richsync.or(subtitle);
        let plain_text = deep_find_string(root, "lyrics_body").and_then(clean_musixmatch_plain);
        let candidate = normalize_candidate(LyricsCandidate {
            source: "musixmatch".to_owned(),
            synced_lrc,
            plain_text,
            artist: track
                .as_ref()
                .and_then(|track| track.artist_name.clone())
                .or_else(|| Some(hints.artist.clone())),
            title: track
                .as_ref()
                .and_then(|track| track.track_name.clone())
                .or_else(|| Some(hints.title.clone())),
            duration_sec: track.as_ref().and_then(|track| track.track_length),
            exact: false,
            query_index,
        });
        Ok(candidate.into_iter().collect())
    }

    async fn macro_payload(
        &self,
        hints: &LyricsHints,
        token: &str,
    ) -> SourceResult<serde_json::Value> {
        let duration = hints.duration_sec.unwrap_or_default().max(0);
        let url = format!(
            "{}/macro.subtitles.get?app_id={}&usertoken={}&namespace=lyrics_richsynched&subtitle_format=lrc&q_track={}&q_artist={}&q_album=&q_duration={duration}&optional_calls=track.richsync&format=json",
            self.base,
            MUSIXMATCH_APP_ID,
            urlencoding::encode(token),
            urlencoding::encode(hints.title.trim()),
            urlencoding::encode(hints.artist.trim()),
        );
        self.send_value(&url).await
    }

    async fn clear_token(&self) {
        *self.token.lock().await = None;
    }

    async fn token(&self) -> SourceResult<String> {
        let mut guard = self.token.lock().await;
        if let Some(cached) = guard.as_ref()
            && cached.expires_at > Instant::now()
        {
            return Ok(cached.token.clone());
        }
        if self.base.is_empty() {
            return Err(SourceError::NotConfigured("musixmatch"));
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        let url = format!(
            "{}/token.get?app_id={MUSIXMATCH_APP_ID}&user_language=en&t={timestamp}",
            self.base
        );
        let token = self
            .send_body::<MxmTokenBody>(&url)
            .await?
            .and_then(|body| body.user_token)
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| SourceError::Invalid("musixmatch token is missing".to_owned()))?;
        if token == "UpgradeOnlyUpgradeOnlyUpgradeOnlyUpgradeOnly" {
            return Err(SourceError::Invalid(
                "musixmatch returned an upgrade-only token".to_owned(),
            ));
        }
        *guard = Some(TokenCache {
            token: token.clone(),
            expires_at: Instant::now() + MUSIXMATCH_TOKEN_TTL,
        });
        Ok(token)
    }

    async fn send_value(&self, url: &str) -> SourceResult<serde_json::Value> {
        let headers = musixmatch_headers(&self.guid)?;
        let bytes = self.fetcher.get_bytes(url, headers).await?;
        serde_json::from_slice(&bytes)
            .map_err(|error| SourceError::Invalid(format!("musixmatch payload: {error}")))
    }

    async fn send_body<T>(&self, url: &str) -> SourceResult<Option<T>>
    where
        T: DeserializeOwned,
    {
        let headers = musixmatch_headers(&self.guid)?;
        let bytes = self.fetcher.get_bytes(url, headers).await?;
        let payload: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| SourceError::Invalid(format!("musixmatch payload: {error}")))?;
        let body = payload
            .get("message")
            .and_then(|message| message.get("body"));
        let Some(body) = body else {
            return Ok(None);
        };
        if body.is_null()
            || body.as_array().is_some_and(Vec::is_empty)
            || body.as_object().is_some_and(serde_json::Map::is_empty)
        {
            return Ok(None);
        }
        serde_json::from_value(body.clone())
            .map(Some)
            .map_err(|error| SourceError::Invalid(format!("musixmatch body: {error}")))
    }
}

fn musixmatch_status(payload: &serde_json::Value) -> Option<u64> {
    payload
        .pointer("/message/header/status_code")
        .and_then(serde_json::Value::as_u64)
}

fn deep_find<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Array(values) => values.iter().find_map(|value| deep_find(value, key)),
        serde_json::Value::Object(values) => values
            .get(key)
            .or_else(|| values.values().find_map(|value| deep_find(value, key))),
        _ => None,
    }
}

fn deep_find_string(value: &serde_json::Value, key: &str) -> Option<String> {
    deep_find(value, key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn richsync_to_lrc(value: String) -> Option<String> {
    let entries: Vec<MxmRichLine> = serde_json::from_str(&value).ok()?;
    let mut lines = Vec::new();
    for entry in entries {
        let Some(timestamp) = entry
            .ts
            .filter(|timestamp| timestamp.is_finite() && *timestamp >= 0.0)
        else {
            continue;
        };
        let words = entry
            .l
            .into_iter()
            .filter_map(|word| word.c)
            .map(|word| word.trim().to_owned())
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>();
        let text = if words.is_empty() {
            entry
                .x
                .map(|text| text.trim().to_owned())
                .unwrap_or_default()
        } else {
            words.join(" ")
        };
        if text.is_empty() {
            continue;
        }
        lines.push((timestamp, text));
    }
    lines.sort_by(|left, right| {
        left.0
            .partial_cmp(&right.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let lrc = lines
        .into_iter()
        .map(|(timestamp, text)| {
            let minutes = (timestamp / 60.0).floor() as i64;
            let seconds = timestamp % 60.0;
            format!("[{minutes:02}:{seconds:05.2}] {text}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    normalize_synced(Some(lrc))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap as AxumHeaders, StatusCode};
    use axum::routing::any;
    use base64::Engine as _;
    use call_relay::Request as RelayRequest;

    use crate::http::{RelayFuture, RelayReply, RelayTransport};

    use super::*;

    #[derive(Clone, Copy)]
    enum MethodMode {
        Found,
        Empty,
        Fail,
    }

    #[derive(Clone, Copy)]
    enum RawMode {
        Hit,
        Fail,
    }

    struct MockRelay {
        method_mode: MethodMode,
        raw_mode: RawMode,
        method_calls: AtomicUsize,
        raw_calls: AtomicUsize,
    }

    struct LiveRelay {
        http: wreq::Client,
    }

    impl RelayTransport for LiveRelay {
        fn call_method(
            &self,
            _method_id: String,
            _script: String,
            _inputs: Bytes,
        ) -> RelayFuture<'_, Bytes> {
            Box::pin(async { Err(SourceError::NotConfigured("live Lua relay")) })
        }

        fn fetch(&self, request: RelayRequest) -> RelayFuture<'_, RelayReply> {
            Box::pin(async move {
                let method = request
                    .method
                    .parse::<wreq::Method>()
                    .map_err(|error| SourceError::Invalid(format!("live method: {error}")))?;
                let mut builder = self.http.request(method, &request.url);
                for (name, value) in request.headers {
                    builder = builder.header(name, value);
                }
                let response = builder
                    .send()
                    .await
                    .map_err(|error| SourceError::Unreachable(error.to_string()))?;
                let status = response.status().as_u16();
                let headers = response
                    .headers()
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.as_str().to_owned(), value.to_owned()))
                    })
                    .collect();
                let body = response
                    .bytes()
                    .await
                    .map_err(|error| SourceError::Unreachable(error.to_string()))?;
                Ok(RelayReply {
                    status,
                    headers,
                    body,
                })
            })
        }
    }

    impl MockRelay {
        fn new(method_mode: MethodMode, raw_mode: RawMode) -> Arc<Self> {
            Arc::new(Self {
                method_mode,
                raw_mode,
                method_calls: AtomicUsize::new(0),
                raw_calls: AtomicUsize::new(0),
            })
        }
    }

    impl RelayTransport for MockRelay {
        fn call_method(
            &self,
            _method_id: String,
            _script: String,
            _inputs: Bytes,
        ) -> RelayFuture<'_, Bytes> {
            self.method_calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async move {
                match self.method_mode {
                    MethodMode::Found => Ok(Bytes::from_static(
                        br#"{"kind":"found","candidates":[{"source":"genius","plain_text":"A sufficiently long exact lyrics body returned by relay Lua","exact":true,"query_index":0}]}"#,
                    )),
                    MethodMode::Empty => Ok(Bytes::from_static(
                        br#"{"kind":"empty","candidates":[]}"#,
                    )),
                    MethodMode::Fail => Err(SourceError::Unreachable(
                        "mock Lua channel failure".to_owned(),
                    )),
                }
            })
        }

        fn fetch(&self, request: RelayRequest) -> RelayFuture<'_, RelayReply> {
            self.raw_calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async move {
                match self.raw_mode {
                    RawMode::Hit => Ok(raw_reply(&request.url)),
                    RawMode::Fail => Err(SourceError::Unreachable(
                        "mock raw relay failure".to_owned(),
                    )),
                }
            })
        }
    }

    fn raw_reply(url: &str) -> RelayReply {
        let body = if url.starts_with("https://lrclib.net/api/search") {
            br#"[{"syncedLyrics":"[00:01.00] A sufficiently long lyrics body from raw relay","plainLyrics":"A sufficiently long lyrics body from raw relay","artistName":"Eminem","trackName":"Lose Yourself","duration":326}]"#.as_slice()
        } else if url.contains("musixmatch.com") && url.contains("token.get") {
            br#"{"message":{"body":{"user_token":"UpgradeOnlyUpgradeOnlyUpgradeOnlyUpgradeOnly"}}}"#
                .as_slice()
        } else if url.starts_with("https://genius.com/api/search/multi") {
            br#"{"response":{"sections":[]}}"#.as_slice()
        } else {
            br#"{}"#.as_slice()
        };
        RelayReply {
            status: 200,
            headers: HashMap::new(),
            body: Bytes::copy_from_slice(body),
        }
    }

    fn sources(relay: Arc<MockRelay>, proxy_url: String) -> Arc<LyricsSources> {
        let http = sc_fingerprint::builder(None)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let fetcher = ExternalFetcher::new_with_transport(http, proxy_url, relay);
        let genius = GeniusService::new(
            fetcher.clone(),
            crate::GeniusCfg {
                access_token: String::new(),
                max_concurrent_scrapes: 8,
            },
        );
        LyricsSources::new(
            fetcher,
            genius,
            "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
        )
    }

    async fn proxy_server() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/", any(proxy_response))
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}/"), calls, server)
    }

    async fn proxy_response(
        State(calls): State<Arc<AtomicUsize>>,
        headers: AxumHeaders,
    ) -> (StatusCode, String) {
        calls.fetch_add(1, Ordering::Relaxed);
        let target = headers
            .get("x-target")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
            .and_then(|value| String::from_utf8(value).ok())
            .unwrap_or_default();
        let body = if target.starts_with("https://lrclib.net/api/search") {
            r#"[{"syncedLyrics":"[00:01.00] A sufficiently long lyrics body from proxy","plainLyrics":"A sufficiently long lyrics body from proxy","artistName":"Eminem","trackName":"Lose Yourself","duration":326}]"#
        } else if target.contains("musixmatch.com") && target.contains("token.get") {
            r#"{"message":{"body":{"user_token":"UpgradeOnlyUpgradeOnlyUpgradeOnlyUpgradeOnly"}}}"#
        } else if target.starts_with("https://genius.com/api/search/multi") {
            r#"{"response":{"sections":[]}}"#
        } else {
            "{}"
        };
        (StatusCode::OK, body.to_owned())
    }

    fn hints() -> LyricsHints {
        LyricsHints {
            title: "Lose Yourself".to_owned(),
            artist: "Eminem".to_owned(),
            duration_sec: Some(326),
            genius_song_id: None,
            genius_url: None,
        }
    }

    #[tokio::test]
    async fn lua_hit_stops_before_raw_relay_and_proxy() {
        let relay = MockRelay::new(MethodMode::Found, RawMode::Fail);
        let service = sources(relay.clone(), String::new());

        let outcome = service.lookup(&hints()).await;

        assert!(matches!(outcome, LyricsLookupOutcome::Found(_)));
        assert_eq!(relay.method_calls.load(Ordering::Relaxed), 1);
        assert_eq!(relay.raw_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn lua_semantic_empty_stops_the_transport_chain() {
        let relay = MockRelay::new(MethodMode::Empty, RawMode::Fail);
        let service = sources(relay.clone(), String::new());

        let outcome = service.lookup(&hints()).await;

        assert_eq!(outcome, LyricsLookupOutcome::CleanMiss);
        assert_eq!(relay.method_calls.load(Ordering::Relaxed), 1);
        assert_eq!(relay.raw_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn lua_failure_falls_back_to_raw_relay_without_proxy() {
        let relay = MockRelay::new(MethodMode::Fail, RawMode::Hit);
        let service = sources(relay.clone(), String::new());

        let outcome = service.lookup(&hints()).await;

        assert!(matches!(outcome, LyricsLookupOutcome::Found(_)));
        assert_eq!(relay.method_calls.load(Ordering::Relaxed), 1);
        assert!(relay.raw_calls.load(Ordering::Relaxed) >= 3);
    }

    #[tokio::test]
    async fn proxy_is_used_only_after_both_relay_paths_fail() {
        let relay = MockRelay::new(MethodMode::Fail, RawMode::Fail);
        let (proxy_url, proxy_calls, server) = proxy_server().await;
        let service = sources(relay.clone(), proxy_url);

        let outcome = service.lookup(&hints()).await;

        server.abort();
        assert!(matches!(outcome, LyricsLookupOutcome::Found(_)));
        assert_eq!(relay.method_calls.load(Ordering::Relaxed), 1);
        assert!(relay.raw_calls.load(Ordering::Relaxed) >= 3);
        assert!(proxy_calls.load(Ordering::Relaxed) >= 3);
    }

    #[test]
    fn richer_synchronized_candidate_wins_deduplication() {
        let candidates = vec![
            LyricsCandidate {
                source: "genius".to_owned(),
                synced_lrc: None,
                plain_text: Some(
                    "A sufficiently long lyrics line for deterministic matching".to_owned(),
                ),
                artist: Some("Eminem".to_owned()),
                title: Some("Lose Yourself".to_owned()),
                duration_sec: None,
                exact: false,
                query_index: 1,
            },
            LyricsCandidate {
                source: "lrclib".to_owned(),
                synced_lrc: Some(
                    "[00:01.00] A sufficiently long lyrics line for deterministic matching"
                        .to_owned(),
                ),
                plain_text: Some(
                    "A sufficiently long lyrics line for deterministic matching".to_owned(),
                ),
                artist: Some("Eminem".to_owned()),
                title: Some("Lose Yourself".to_owned()),
                duration_sec: Some(326),
                exact: false,
                query_index: 1,
            },
        ];

        let selected = select_candidate(&hints(), candidates).unwrap();

        assert_eq!(selected.source, "lrclib");
        assert!(selected.synced_lrc.is_some());
    }

    #[test]
    fn lrc_metadata_is_not_embedded_as_plain_text() {
        let value = "[ar:Eminem]\n[00:01.00] first line\n[00:02.00] second line";

        assert_eq!(strip_lrc_timestamps(value), "first line\nsecond line");
    }

    #[test]
    fn malformed_synchronized_text_is_rejected() {
        let candidate = normalize_candidate(LyricsCandidate {
            source: "lrclib".to_owned(),
            synced_lrc: Some("not synchronized lyrics at all".to_owned()),
            plain_text: None,
            artist: None,
            title: None,
            duration_sec: None,
            exact: false,
            query_index: 1,
        });

        assert!(candidate.is_none());
    }

    #[test]
    fn duration_and_artist_mismatch_reject_candidates() {
        let candidate = LyricsCandidate {
            source: "lrclib".to_owned(),
            synced_lrc: None,
            plain_text: Some(
                "A sufficiently long lyrics line for deterministic matching".to_owned(),
            ),
            artist: Some("Someone Else".to_owned()),
            title: Some("Lose Yourself".to_owned()),
            duration_sec: Some(60),
            exact: false,
            query_index: 1,
        };

        assert!(!matches_hints(&hints(), &candidate));
    }

    #[test]
    fn richsync_is_converted_to_valid_lrc() {
        let value =
            r#"[{"ts":1.5,"l":[{"c":"hello"},{"c":"world"}]},{"ts":3.0,"x":"second line"}]"#;

        let lrc = richsync_to_lrc(value.to_owned()).unwrap();

        assert_eq!(lrc, "[00:01.50] hello world\n[00:03.00] second line");
    }

    #[test]
    fn commercial_footer_is_removed_without_discarding_lyrics() {
        let value = "A sufficiently long real lyrics line\n******* This Lyrics is NOT for Commercial use *******";

        let cleaned = clean_musixmatch_plain(value.to_owned()).unwrap();

        assert_eq!(cleaned, "A sufficiently long real lyrics line");
    }

    #[test]
    fn macro_payload_fields_are_found_recursively() {
        let payload = serde_json::json!({
            "macro_calls": {
                "matcher.track.get": {
                    "message": { "body": { "track": { "track_name": "Track" } } }
                },
                "track.subtitles.get": {
                    "message": { "body": { "subtitle": { "subtitle_body": "[00:01.00] line" } } }
                }
            }
        });

        assert!(deep_find(&payload, "track").is_some());
        assert_eq!(
            deep_find_string(&payload, "subtitle_body").as_deref(),
            Some("[00:01.00] line")
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_musixmatch_returns_lyrics() {
        let http = sc_fingerprint::builder(None)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let relay = Arc::new(LiveRelay { http: http.clone() });
        let fetcher = ExternalFetcher::new_with_transport(http, String::new(), relay);
        let musixmatch = MusixmatchLyrics::new(
            fetcher,
            "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
        );
        let fixtures = [
            ("Eminem", "Lose Yourself", 326),
            ("Adele", "Hello", 295),
            ("Billie Eilish", "BIRDS OF A FEATHER", 210),
            ("Linkin Park", "Numb", 187),
            ("Rick Astley", "Never Gonna Give You Up", 213),
        ];
        let mut has_plain = false;
        let mut has_synced = false;
        for (artist, title, duration_sec) in fixtures {
            let hints = LyricsHints {
                artist: artist.to_owned(),
                title: title.to_owned(),
                duration_sec: Some(duration_sec),
                genius_song_id: None,
                genius_url: None,
            };
            let candidates = musixmatch
                .search(&hints, 1)
                .await
                .expect("musixmatch lookup");
            has_plain |= candidates
                .iter()
                .any(|candidate| candidate.plain_text.is_some());
            has_synced |= candidates
                .iter()
                .any(|candidate| candidate.synced_lrc.is_some());
            if has_plain && has_synced {
                break;
            }
        }

        assert!(has_plain);
        assert!(has_synced);
    }

    #[test]
    fn heuristic_queries_are_canonical_and_bounded() {
        let queries = heuristic_queries("Eminem", "Eminem - Lose Yourself");
        let unique = queries
            .iter()
            .map(|query| query.to_lowercase())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(queries.len(), unique.len());
        assert!(queries.len() <= MAX_QUERIES);
    }
}
