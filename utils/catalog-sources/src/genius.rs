use std::sync::Arc;
use std::time::Duration;

use futures::future::try_join_all;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tracing::warn;
use wreq::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};

use crate::error::{SourceError, SourceResult};
use crate::http::ExternalFetcher;
use crate::throttle::Throttle;

#[derive(Clone)]
pub struct GeniusCfg {
    pub access_token: String,
    pub max_concurrent_scrapes: usize,
}

const GENIUS_API: &str = "https://api.genius.com";
const GENIUS_WEB_API: &str = "https://genius.com/api";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

const API_THROTTLE_MS: u64 = 0;
const WEB_DIRECT_THROTTLE_MS: u64 = 0;
const GENIUS_PAGINATE: &str = call_lua_macros::lua_script!("catalog_methods/genius_paginate.lua");
const GENIUS_PAGINATE_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

static RE_OPEN: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r#"(?i)<div\b[^>]*\bdata-lyrics-container="true"[^>]*>"#).ok());
static RE_BR: Lazy<Option<Regex>> = Lazy::new(|| Regex::new(r"(?i)<br\s*/?>").ok());
static RE_TAGS: Lazy<Option<Regex>> = Lazy::new(|| Regex::new(r"<[^>]+>").ok());
static RE_LEAD_CONTRIB: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"(?i)^\d+\s*Contributors").ok());
static RE_LEAD_LYRICS: Lazy<Option<Regex>> = Lazy::new(|| Regex::new(r"(?i)^[^\n]*?Lyrics").ok());
static RE_LEAD_TEXT_PESN: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"(?i)^\[Текст песни.*?\]").ok());

#[derive(Debug, Clone)]
pub struct GeniusCandidate {
    pub plain_text: String,
    pub artist_guess: Option<String>,
    pub title_guess: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResp {
    response: Option<SearchRespBody>,
}
#[derive(Debug, Deserialize)]
struct SearchRespBody {
    sections: Option<Vec<SearchSection>>,
}
#[derive(Debug, Deserialize)]
struct SearchSection {
    #[serde(rename = "type")]
    type_: String,
    hits: Option<Vec<SearchHit>>,
}
#[derive(Debug, Deserialize)]
struct SearchHit {
    result: Option<SearchHitResult>,
}
#[derive(Debug, Deserialize)]
struct SearchHitResult {
    #[serde(default)]
    id: Option<i64>,
    url: Option<String>,
    title: Option<String>,
    primary_artist: Option<PrimaryArtist>,
    #[serde(default)]
    featured_artists: Option<Vec<PrimaryArtist>>,
}

#[derive(Debug, Deserialize)]
struct ApiSearchResp {
    #[serde(default)]
    response: Option<ApiSearchBody>,
}

#[derive(Debug, Deserialize)]
struct ApiSearchBody {
    #[serde(default)]
    hits: Option<Vec<SearchHit>>,
}
#[derive(Debug, Deserialize)]
struct PrimaryArtist {
    #[serde(default)]
    id: Option<i64>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtistSongsResp {
    #[serde(default)]
    response: Option<ArtistSongsBody>,
}

#[derive(Debug, Deserialize)]
struct ArtistSongsBody {
    #[serde(default)]
    songs: Option<Vec<ArtistSong>>,
}

#[derive(Debug, Deserialize)]
struct ArtistSong {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    primary_artist: Option<PrimaryArtist>,
    #[serde(default)]
    featured_artists: Option<Vec<PrimaryArtist>>,
}

#[derive(Debug, Deserialize)]
struct ArtistAlbumsResp {
    #[serde(default)]
    response: Option<ArtistAlbumsBody>,
}

#[derive(Debug, Deserialize)]
struct ArtistAlbumsBody {
    #[serde(default)]
    albums: Option<Vec<AlbumPayload>>,
    #[serde(default)]
    next_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AlbumTracksResp {
    #[serde(default)]
    response: Option<AlbumTracksBody>,
}

#[derive(Debug, Deserialize)]
struct AlbumTracksBody {
    #[serde(default)]
    tracks: Option<Vec<AlbumTrackEntry>>,
    #[serde(default)]
    next_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AlbumTrackEntry {
    #[serde(default)]
    number: Option<i32>,
    #[serde(default)]
    song: Option<AlbumTrackSong>,
}

#[derive(Debug, Deserialize)]
struct AlbumTrackSong {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    primary_artist: Option<PrimaryArtist>,
    #[serde(default)]
    featured_artists: Option<Vec<PrimaryArtist>>,
}

#[derive(Debug, Deserialize)]
struct ArtistResp {
    #[serde(default)]
    response: Option<ArtistRespBody>,
}

#[derive(Debug, Deserialize)]
struct ArtistRespBody {
    #[serde(default)]
    artist: Option<ArtistPayload>,
}

#[derive(Debug, Deserialize)]
struct ArtistPayload {
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    instagram_name: Option<String>,
    #[serde(default)]
    twitter_name: Option<String>,
    #[serde(default)]
    facebook_name: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SongResp {
    #[serde(default)]
    response: Option<SongRespBody>,
}

#[derive(Debug, Deserialize)]
struct SongRespBody {
    #[serde(default)]
    song: Option<SongPayload>,
}

#[derive(Debug, Deserialize)]
struct SongPayload {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    album: Option<AlbumPayload>,
    #[serde(default)]
    release_date_components: Option<ReleaseDate>,
}

#[derive(Debug, Deserialize)]
struct AlbumPayload {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cover_art_url: Option<String>,
    #[serde(default)]
    release_date_components: Option<ReleaseDate>,
}

#[derive(Debug, Deserialize)]
struct ReleaseDate {
    #[serde(default)]
    year: Option<i32>,
    #[serde(default)]
    month: Option<u32>,
    #[serde(default)]
    day: Option<u32>,
}

impl ReleaseDate {
    fn full_date(&self) -> Option<chrono::NaiveDate> {
        let y = self.year?;
        let m = self.month?.clamp(1, 12);
        let d = self.day?.clamp(1, 31);
        chrono::NaiveDate::from_ymd_opt(y, m, d)
    }
}

#[derive(Debug, Clone)]
pub struct GeniusArtistRef {
    pub genius_artist_id: Option<i64>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct GeniusSongMeta {
    pub genius_song_id: Option<i64>,
    pub title: String,
    pub primary_artist: Option<GeniusArtistRef>,
    pub featured: Vec<GeniusArtistRef>,
}

#[derive(Debug, Clone)]
pub struct GeniusArtistDetails {
    pub avatar_url: Option<String>,
    pub instagram: Option<String>,
    pub twitter: Option<String>,
    pub facebook: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GeniusAlbumRef {
    pub genius_album_id: i64,
    pub name: String,
    pub year: Option<i16>,
    pub release_date: Option<chrono::NaiveDate>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GeniusSongDetails {
    pub url: Option<String>,
    pub album: Option<GeniusAlbumRef>,
    pub year: Option<i16>,
    pub release_date: Option<chrono::NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct GeniusAlbumTrack {
    pub genius_song_id: i64,
    pub title: String,
    pub position: Option<i32>,
    pub primary_artist: Option<GeniusArtistRef>,
    pub featured: Vec<GeniusArtistRef>,
}

#[derive(Serialize)]
struct PaginationInput<'a> {
    kind: &'a str,
    id: i64,
    start_page: u32,
    per_page: u32,
    max_pages: usize,
}

#[derive(Deserialize)]
struct PaginationEnvelope<T> {
    kind: String,
    items: Vec<T>,
    complete: bool,
    next_page: u32,
}

struct PaginationWindow<T> {
    items: Vec<T>,
    complete: bool,
    next_page: u32,
}

pub struct GeniusService {
    fetcher: Arc<ExternalFetcher>,
    cfg: GeniusCfg,
    api_throttle: Arc<Throttle>,
    web_throttle: Arc<Throttle>,
    scrape_sem: Arc<Semaphore>,
}

impl GeniusService {
    pub fn new(fetcher: Arc<ExternalFetcher>, cfg: GeniusCfg) -> Arc<Self> {
        let scrape_sem = Arc::new(Semaphore::new(cfg.max_concurrent_scrapes.max(1)));
        Arc::new(Self {
            fetcher,
            cfg,
            api_throttle: Throttle::new(Duration::from_millis(API_THROTTLE_MS)),
            web_throttle: Throttle::new(Duration::from_millis(WEB_DIRECT_THROTTLE_MS)),
            scrape_sem,
        })
    }

    fn has_token(&self) -> bool {
        !self.cfg.access_token.is_empty()
    }

    fn json_headers(&self, with_bearer: bool) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        if with_bearer
            && self.has_token()
            && let Ok(value) = format!("Bearer {}", self.cfg.access_token).parse()
        {
            headers.insert(AUTHORIZATION, value);
        }
        headers
    }

    fn html_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        headers
    }

    fn web_api(&self, path: &str) -> String {
        format!("{GENIUS_WEB_API}{path}")
    }

    fn api(&self, path: &str) -> String {
        format!("{GENIUS_API}{path}")
    }

    async fn fetch_json_strict<T>(&self, url: &str, label: &str) -> SourceResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let with_bearer = url.starts_with(GENIUS_API);
        let headers = self.json_headers(with_bearer);
        let _permit = self.scrape_sem.acquire().await.ok();
        let bytes = if with_bearer {
            self.fetcher.get_api(url, headers, &self.api_throttle).await
        } else {
            self.fetcher
                .get_scrape(url, headers, &self.web_throttle)
                .await
        }?;
        serde_json::from_slice::<T>(&bytes).map_err(|e| {
            let head: String = String::from_utf8_lossy(&bytes).chars().take(80).collect();
            warn!(url, label, error = %e, head = %head, "genius parse failed");
            SourceError::Invalid(format!("genius {label}: {e}"))
        })
    }

    async fn fetch_json<T>(&self, url: &str, label: &str) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        match self.fetch_json_strict(url, label).await {
            Ok(v) => Some(v),
            Err(SourceError::Invalid(_)) => None,
            Err(e) => {
                warn!(url, label, error = %e, "genius fetch failed");
                None
            }
        }
    }

    async fn fetch_html_strict(&self, url: &str) -> SourceResult<String> {
        let _permit = self.scrape_sem.acquire().await.ok();
        let bytes = self
            .fetcher
            .get_scrape(url, self.html_headers(), &self.web_throttle)
            .await?;
        let html = String::from_utf8(bytes.to_vec())
            .map_err(|error| SourceError::Invalid(format!("genius html utf8: {error}")))?;
        let lowercase = html.to_lowercase();
        if lowercase.contains("cf-challenge")
            || lowercase.contains("captcha")
            || lowercase.contains("just a moment")
        {
            return Err(SourceError::Invalid(
                "genius returned a challenge page".to_owned(),
            ));
        }
        Ok(html)
    }

    async fn paginate<T>(
        &self,
        kind: &str,
        id: i64,
        start_page: u32,
        per_page: u32,
        max_pages: usize,
    ) -> SourceResult<PaginationWindow<T>>
    where
        T: DeserializeOwned,
    {
        let inputs = serde_json::to_vec(&PaginationInput {
            kind,
            id,
            start_page,
            per_page,
            max_pages,
        })
        .map_err(|error| SourceError::Invalid(format!("genius pagination input: {error}")))?;
        let output = self
            .fetcher
            .call_method(
                "genius.paginate",
                GENIUS_PAGINATE,
                inputs.into(),
                GENIUS_PAGINATE_OUTPUT_LIMIT,
            )
            .await?;
        let envelope: PaginationEnvelope<T> = serde_json::from_slice(&output)
            .map_err(|error| SourceError::Invalid(format!("genius pagination output: {error}")))?;
        if !matches!(envelope.kind.as_str(), "found" | "empty") {
            return Err(SourceError::Invalid(
                "genius pagination returned an unknown outcome".to_owned(),
            ));
        }
        if envelope.kind == "empty" && !envelope.items.is_empty() {
            return Err(SourceError::Invalid(
                "genius pagination returned contradictory output".to_owned(),
            ));
        }
        Ok(PaginationWindow {
            items: envelope.items,
            complete: envelope.complete,
            next_page: envelope.next_page,
        })
    }

    pub async fn list_artist_songs_window(
        &self,
        genius_id: i64,
        starting_offset: u32,
        per_page: u32,
        max_pages: usize,
    ) -> SourceResult<(Vec<GeniusSongMeta>, u32)> {
        let per_page = per_page.clamp(1, 50);
        let start_page = starting_offset / per_page + 1;
        match self
            .paginate::<ArtistSong>(
                "artist_songs",
                genius_id,
                start_page,
                per_page,
                max_pages.clamp(1, 20),
            )
            .await
        {
            Ok(window) => {
                let songs = window.items.into_iter().filter_map(map_song).collect();
                let next_offset = if window.complete {
                    0
                } else {
                    window.next_page.saturating_sub(1).saturating_mul(per_page)
                };
                Ok((songs, next_offset))
            }
            Err(_) => {
                let mut songs = Vec::new();
                let mut offset = starting_offset;
                for _ in 0..max_pages.max(1) {
                    let page = offset / per_page + 1;
                    let batch = self.list_artist_songs(genius_id, page, per_page).await?;
                    let count = u32::try_from(batch.len()).unwrap_or(u32::MAX);
                    songs.extend(batch);
                    if count < per_page {
                        return Ok((songs, 0));
                    }
                    offset = offset.saturating_add(count);
                }
                Ok((songs, offset))
            }
        }
    }

    pub async fn list_artist_songs(
        &self,
        genius_id: i64,
        page: u32,
        per_page: u32,
    ) -> SourceResult<Vec<GeniusSongMeta>> {
        let per = per_page.clamp(1, 50);
        let pg = page.max(1);
        let path = format!("/artists/{genius_id}/songs?per_page={per}&page={pg}&sort=popularity");
        let url = self.web_api(&path);
        let parsed: ArtistSongsResp = self.fetch_json_strict(&url, "artist songs").await?;
        Ok(parsed
            .response
            .map(|r| r.songs.unwrap_or_default())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| {
                let title = s.title?;
                let primary = s.primary_artist.as_ref().and_then(map_artist);
                let featured = s
                    .featured_artists
                    .as_deref()
                    .map(|arr| arr.iter().filter_map(map_artist).collect())
                    .unwrap_or_default();
                Some(GeniusSongMeta {
                    genius_song_id: s.id,
                    title,
                    primary_artist: primary,
                    featured,
                })
            })
            .collect())
    }

    pub async fn list_artist_albums_window(
        &self,
        genius_id: i64,
        per_page: u32,
        max_pages: usize,
    ) -> SourceResult<Vec<GeniusAlbumRef>> {
        let per_page = per_page.clamp(1, 50);
        match self
            .paginate::<AlbumPayload>(
                "artist_albums",
                genius_id,
                1,
                per_page,
                max_pages.clamp(1, 20),
            )
            .await
        {
            Ok(window) => Ok(window.items.into_iter().filter_map(map_album).collect()),
            Err(_) => {
                let mut albums = Vec::new();
                for page in 1..=u32::try_from(max_pages.max(1)).unwrap_or(u32::MAX) {
                    let (batch, has_more) =
                        self.list_artist_albums(genius_id, page, per_page).await?;
                    albums.extend(batch);
                    if !has_more {
                        break;
                    }
                }
                Ok(albums)
            }
        }
    }

    pub async fn list_artist_albums(
        &self,
        genius_id: i64,
        page: u32,
        per_page: u32,
    ) -> SourceResult<(Vec<GeniusAlbumRef>, bool)> {
        let per = per_page.clamp(1, 50);
        let pg = page.max(1);
        let url = self.web_api(&format!(
            "/artists/{genius_id}/albums?per_page={per}&page={pg}"
        ));
        let parsed: ArtistAlbumsResp = self.fetch_json_strict(&url, "artist albums").await?;
        let body = parsed.response.unwrap_or(ArtistAlbumsBody {
            albums: None,
            next_page: None,
        });
        let has_more = body.next_page.is_some();
        let out = body
            .albums
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| {
                let id = a.id?;
                let name = a
                    .name
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())?;
                let rd = a.release_date_components;
                let year = rd
                    .as_ref()
                    .and_then(|d| d.year)
                    .and_then(|y| i16::try_from(y).ok());
                let release_date = rd.as_ref().and_then(ReleaseDate::full_date);
                Some(GeniusAlbumRef {
                    genius_album_id: id,
                    name,
                    year,
                    release_date,
                    cover_url: a.cover_art_url.filter(|s| !s.is_empty()),
                })
            })
            .collect();
        Ok((out, has_more))
    }

    pub async fn list_album_tracks_window(
        &self,
        genius_album_id: i64,
        per_page: u32,
        max_pages: usize,
    ) -> SourceResult<Vec<GeniusAlbumTrack>> {
        let per_page = per_page.clamp(1, 50);
        match self
            .paginate::<AlbumTrackEntry>(
                "album_tracks",
                genius_album_id,
                1,
                per_page,
                max_pages.clamp(1, 20),
            )
            .await
        {
            Ok(window) => Ok(window
                .items
                .into_iter()
                .filter_map(map_album_track)
                .collect()),
            Err(_) => {
                let mut tracks = Vec::new();
                for page in 1..=u32::try_from(max_pages.max(1)).unwrap_or(u32::MAX) {
                    let (batch, has_more) = self
                        .list_album_tracks(genius_album_id, page, per_page)
                        .await?;
                    tracks.extend(batch);
                    if !has_more {
                        break;
                    }
                }
                Ok(tracks)
            }
        }
    }

    pub async fn list_album_tracks(
        &self,
        genius_album_id: i64,
        page: u32,
        per_page: u32,
    ) -> SourceResult<(Vec<GeniusAlbumTrack>, bool)> {
        let per = per_page.clamp(1, 50);
        let pg = page.max(1);
        let url = self.web_api(&format!(
            "/albums/{genius_album_id}/tracks?per_page={per}&page={pg}"
        ));
        let parsed: AlbumTracksResp = self.fetch_json_strict(&url, "album tracks").await?;
        let body = parsed.response.unwrap_or(AlbumTracksBody {
            tracks: None,
            next_page: None,
        });
        let has_more = body.next_page.is_some();
        let tracks = body
            .tracks
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| {
                let song = t.song?;
                let id = song.id?;
                let title = song
                    .title
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())?;
                let primary = song.primary_artist.as_ref().and_then(map_artist);
                let featured = song
                    .featured_artists
                    .as_deref()
                    .map(|arr| arr.iter().filter_map(map_artist).collect())
                    .unwrap_or_default();
                Some(GeniusAlbumTrack {
                    genius_song_id: id,
                    title,
                    position: t.number,
                    primary_artist: primary,
                    featured,
                })
            })
            .collect();
        Ok((tracks, has_more))
    }

    pub async fn lookup_song(&self, genius_song_id: i64) -> Option<GeniusSongDetails> {
        let path = format!("/songs/{genius_song_id}");
        let url = self.web_api(&path);
        let parsed: SongResp = self.fetch_json(&url, "song").await?;
        let song = parsed.response.and_then(|r| r.song)?;
        let song_rd = song.release_date_components;
        let song_year = song_rd
            .as_ref()
            .and_then(|d| d.year)
            .and_then(|y| i16::try_from(y).ok());
        let song_date = song_rd.as_ref().and_then(ReleaseDate::full_date);
        let album = song.album.and_then(|a| {
            let id = a.id?;
            let name = a
                .name
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())?;
            let rd = a.release_date_components;
            let year = rd
                .as_ref()
                .and_then(|d| d.year)
                .and_then(|y| i16::try_from(y).ok());
            let release_date = rd.as_ref().and_then(ReleaseDate::full_date);
            Some(GeniusAlbumRef {
                genius_album_id: id,
                name,
                year,
                release_date,
                cover_url: a.cover_art_url,
            })
        });
        Some(GeniusSongDetails {
            url: song.url.filter(|s| !s.is_empty()),
            album,
            year: song_year,
            release_date: song_date,
        })
    }

    pub async fn lyrics_by_url(&self, url: &str) -> Option<String> {
        self.lyrics_by_url_strict(url).await.ok().flatten()
    }

    pub async fn lyrics_by_url_strict(&self, url: &str) -> SourceResult<Option<String>> {
        let html = self.fetch_html_strict(url).await?;
        Ok(parse_lyrics_html(&html))
    }

    pub async fn lyrics_by_song_id(&self, genius_song_id: i64) -> Option<String> {
        self.lyrics_by_song_id_strict(genius_song_id)
            .await
            .ok()
            .flatten()
    }

    pub async fn lyrics_by_song_id_strict(
        &self,
        genius_song_id: i64,
    ) -> SourceResult<Option<String>> {
        let path = format!("/songs/{genius_song_id}");
        let url = self.web_api(&path);
        let parsed: SongResp = self.fetch_json_strict(&url, "song lyrics").await?;
        let Some(song_url) = parsed
            .response
            .and_then(|response| response.song)
            .and_then(|song| song.url)
            .filter(|url| !url.is_empty())
        else {
            return Ok(None);
        };
        self.lyrics_by_url_strict(&song_url).await
    }

    pub async fn lookup_artist(&self, genius_id: i64) -> Option<GeniusArtistDetails> {
        let path = format!("/artists/{genius_id}");
        let url = self.web_api(&path);
        let parsed: ArtistResp = self.fetch_json(&url, "artist").await?;
        let a = parsed.response.and_then(|r| r.artist)?;
        Some(GeniusArtistDetails {
            avatar_url: a.image_url,
            instagram: a.instagram_name.filter(|s| !s.is_empty()),
            twitter: a.twitter_name.filter(|s| !s.is_empty()),
            facebook: a.facebook_name.filter(|s| !s.is_empty()),
            url: a.url,
        })
    }

    pub async fn search_song_meta(
        &self,
        q: &str,
        limit: usize,
    ) -> SourceResult<Vec<GeniusSongMeta>> {
        let mut transport: Option<SourceError> = None;
        if self.has_token() {
            let url = self.api(&format!("/search?q={}", urlencoding::encode(q)));
            match self
                .fetch_json_strict::<ApiSearchResp>(&url, "api-search")
                .await
            {
                Ok(parsed) => {
                    let hits = map_api_song_hits(parsed, limit);
                    if !hits.is_empty() {
                        return Ok(hits);
                    }
                }
                Err(e) => transport = Some(e),
            }
        }
        let url = self.web_api(&format!("/search/multi?q={}", urlencoding::encode(q)));
        match self.fetch_json_strict::<SearchResp>(&url, "search").await {
            Ok(data) => Ok(map_web_song_hits(&data, limit)),
            Err(e) => Err(transport.unwrap_or(e)),
        }
    }

    pub async fn search_artist(
        &self,
        name: &str,
        limit: usize,
    ) -> SourceResult<Vec<GeniusArtistRef>> {
        let songs = self.search_song_meta(name, limit).await?;
        let mut seen: Vec<i64> = Vec::new();
        let mut artists: Vec<GeniusArtistRef> = Vec::new();
        for song in songs {
            let Some(artist) = song.primary_artist else {
                continue;
            };
            let Some(id) = artist.genius_artist_id else {
                continue;
            };
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            artists.push(artist);
        }
        Ok(artists)
    }

    pub async fn search_by_query(&self, q: &str, limit: usize) -> Vec<GeniusCandidate> {
        self.search_by_query_strict(q, limit)
            .await
            .unwrap_or_default()
    }

    pub async fn search_by_query_strict(
        &self,
        q: &str,
        limit: usize,
    ) -> SourceResult<Vec<GeniusCandidate>> {
        let hits = self.collect_lyric_hits_strict(q, limit).await?;
        let scrapes = hits.into_iter().take(limit).map(|hit| async move {
            let html = self.fetch_html_strict(&hit.url).await?;
            Ok::<_, SourceError>(parse_lyrics_html(&html).map(|plain_text| GeniusCandidate {
                plain_text,
                artist_guess: hit.artist,
                title_guess: hit.title,
            }))
        });
        Ok(try_join_all(scrapes).await?.into_iter().flatten().collect())
    }

    async fn collect_lyric_hits_strict(
        &self,
        q: &str,
        limit: usize,
    ) -> SourceResult<Vec<LyricHit>> {
        let url = self.web_api(&format!("/search/multi?q={}", urlencoding::encode(q)));
        let data: SearchResp = self.fetch_json_strict(&url, "lyrics search").await?;
        let mut out = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();
        let sections = data
            .response
            .as_ref()
            .and_then(|response| response.sections.as_ref());
        let Some(sections) = sections else {
            return Ok(out);
        };
        for section in sections {
            if section.type_ != "song" {
                continue;
            }
            let Some(hits) = &section.hits else {
                continue;
            };
            for hit in hits {
                let Some(result) = &hit.result else {
                    continue;
                };
                let Some(url) = &result.url else {
                    continue;
                };
                if !seen_urls.insert(url.clone()) {
                    continue;
                }
                out.push(LyricHit {
                    url: url.clone(),
                    artist: result
                        .primary_artist
                        .as_ref()
                        .and_then(|artist| artist.name.clone()),
                    title: result.title.clone(),
                });
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
struct LyricHit {
    url: String,
    artist: Option<String>,
    title: Option<String>,
}

fn map_album(album: AlbumPayload) -> Option<GeniusAlbumRef> {
    let id = album.id?;
    let name = album
        .name
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())?;
    let release_date = album.release_date_components;
    let year = release_date
        .as_ref()
        .and_then(|date| date.year)
        .and_then(|year| i16::try_from(year).ok());
    let full_date = release_date.as_ref().and_then(ReleaseDate::full_date);
    Some(GeniusAlbumRef {
        genius_album_id: id,
        name,
        year,
        release_date: full_date,
        cover_url: album.cover_art_url.filter(|url| !url.is_empty()),
    })
}

fn map_album_track(entry: AlbumTrackEntry) -> Option<GeniusAlbumTrack> {
    let song = entry.song?;
    let id = song.id?;
    let title = song
        .title
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())?;
    let primary = song.primary_artist.as_ref().and_then(map_artist);
    let featured = song
        .featured_artists
        .as_deref()
        .map(|artists| artists.iter().filter_map(map_artist).collect())
        .unwrap_or_default();
    Some(GeniusAlbumTrack {
        genius_song_id: id,
        title,
        position: entry.number,
        primary_artist: primary,
        featured,
    })
}

fn map_song(song: ArtistSong) -> Option<GeniusSongMeta> {
    let title = song.title?;
    let primary_artist = song.primary_artist.as_ref().and_then(map_artist);
    let featured = song
        .featured_artists
        .as_deref()
        .map(|artists| artists.iter().filter_map(map_artist).collect())
        .unwrap_or_default();
    Some(GeniusSongMeta {
        genius_song_id: song.id,
        title,
        primary_artist,
        featured,
    })
}

fn map_artist(a: &PrimaryArtist) -> Option<GeniusArtistRef> {
    let name = a.name.as_ref()?.trim();
    if name.is_empty() {
        return None;
    }
    Some(GeniusArtistRef {
        genius_artist_id: a.id,
        name: name.to_string(),
    })
}

fn map_api_song_hits(parsed: ApiSearchResp, limit: usize) -> Vec<GeniusSongMeta> {
    let hits = parsed.response.and_then(|r| r.hits).unwrap_or_default();
    hits.into_iter()
        .take(limit)
        .filter_map(|h| {
            let result = h.result?;
            let title = result.title?;
            let primary = result.primary_artist.as_ref().and_then(map_artist);
            let featured = result
                .featured_artists
                .as_deref()
                .map(|arr| arr.iter().filter_map(map_artist).collect())
                .unwrap_or_default();
            Some(GeniusSongMeta {
                genius_song_id: result.id,
                title,
                primary_artist: primary,
                featured,
            })
        })
        .collect()
}

fn map_web_song_hits(data: &SearchResp, limit: usize) -> Vec<GeniusSongMeta> {
    let mut out = Vec::new();
    let sections = data.response.as_ref().and_then(|r| r.sections.as_ref());
    let Some(secs) = sections else { return out };
    for section in secs {
        if section.type_ != "song" {
            continue;
        }
        let Some(hits) = &section.hits else { continue };
        for hit in hits.iter().take(limit) {
            let Some(result) = &hit.result else { continue };
            let Some(title) = result.title.clone() else {
                continue;
            };
            let primary = result.primary_artist.as_ref().and_then(map_artist);
            let featured = result
                .featured_artists
                .as_deref()
                .map(|arr| arr.iter().filter_map(map_artist).collect())
                .unwrap_or_default();
            out.push(GeniusSongMeta {
                genius_song_id: result.id,
                title,
                primary_artist: primary,
                featured,
            });
        }
    }
    out
}

fn parse_lyrics_html(html: &str) -> Option<String> {
    let open = RE_OPEN.as_ref()?;
    let br = RE_BR.as_ref()?;
    let tags = RE_TAGS.as_ref()?;
    let lead_contributors = RE_LEAD_CONTRIB.as_ref()?;
    let lead_lyrics = RE_LEAD_LYRICS.as_ref()?;
    let lead_text = RE_LEAD_TEXT_PESN.as_ref()?;
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    while let Some(m) = open.find_at(html, cursor) {
        let start = m.end();
        if let Some(inner) = extract_balanced_div_content(html, start) {
            parts.push(inner);
        }
        cursor = m.end();
        if cursor >= html.len() {
            break;
        }
    }
    if parts.is_empty() {
        return None;
    }

    let mut text = parts.join("\n");
    text = br.replace_all(&text, "\n").into_owned();
    text = tags.replace_all(&text, "").into_owned();
    text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&quot;", "\"");

    text = lead_contributors.replace(&text, "").into_owned();
    text = lead_lyrics.replace(&text, "").into_owned();
    text = lead_text.replace(&text, "").into_owned();
    let trimmed = text.trim().to_string();
    if trimmed.len() > 20 {
        Some(trimmed)
    } else {
        None
    }
}

fn extract_balanced_div_content(html: &str, start_pos: usize) -> Option<String> {
    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut depth = 1i32;
    let mut pos = start_pos;
    while pos < len && depth > 0 {
        let next_open = find_subseq(bytes, pos, b"<div");
        let next_close = find_subseq(bytes, pos, b"</div");
        let nc = next_close?;
        match next_open {
            Some(no) if no < nc => {
                let after_idx = no + 4;
                let after = if after_idx < len { bytes[after_idx] } else { 0 };
                if matches!(after, b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
                    depth += 1;
                }
                pos = no + 4;
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return Some(html[start_pos..nc].to_string());
                }
                pos = nc + 5;
            }
        }
    }
    None
}

fn find_subseq(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    let n = needle.len();
    let mut i = from;
    while i + n <= haystack.len() {
        if &haystack[i..i + n] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_client() -> Arc<GeniusService> {
        let http = sc_fingerprint::builder(None)
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        let fetcher = ExternalFetcher::new(http, String::new(), None);
        GeniusService::new(
            fetcher,
            GeniusCfg {
                access_token: String::new(),
                max_concurrent_scrapes: 50,
            },
        )
    }

    #[tokio::test]
    #[ignore]
    async fn live_search_psychosis_x_ray() {
        let svc = build_client();
        let candidates = svc
            .search_song_meta("Psychosis x-ray", 5)
            .await
            .expect("genius reachable");
        assert!(!candidates.is_empty(), "Genius returned no candidates");
        let psychosis = candidates
            .iter()
            .find(|c| {
                c.primary_artist
                    .as_ref()
                    .map(|a| a.name.to_lowercase() == "psychosis")
                    .unwrap_or(false)
            })
            .expect("expected Psychosis as primary artist in results");
        assert_eq!(psychosis.title.to_lowercase(), "x-ray");
    }

    #[tokio::test]
    #[ignore]
    async fn live_list_psychosis_albums() {
        let svc = build_client();
        let (albums, _has_more) = svc.list_artist_albums(3401261, 1, 20).await.unwrap();
        assert!(
            albums.len() >= 5,
            "expected several albums, got {}",
            albums.len()
        );
        let names: Vec<String> = albums.iter().map(|a| a.name.to_lowercase()).collect();
        assert!(
            names.iter().any(|n| n.contains("euphoria")),
            "euphoria not in {:?}",
            names
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_album_tracks_euphoria() {
        let svc = build_client();
        let (tracks, _) = svc.list_album_tracks(1222807, 1, 50).await.unwrap();
        assert!(tracks.len() >= 5);
        assert!(tracks.iter().all(|t| t.genius_song_id > 0));
    }

    #[tokio::test]
    #[ignore]
    async fn live_search_eminem_lose_yourself() {
        let svc = build_client();
        let candidates = svc
            .search_song_meta("Eminem Lose Yourself", 5)
            .await
            .expect("genius reachable");
        assert!(!candidates.is_empty());
        assert!(candidates.iter().any(|c| {
            c.primary_artist
                .as_ref()
                .map(|a| a.name.to_lowercase() == "eminem")
                .unwrap_or(false)
        }));
    }
}
