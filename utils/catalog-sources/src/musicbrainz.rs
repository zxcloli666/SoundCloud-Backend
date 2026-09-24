use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use wreq::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, USER_AGENT};

use crate::error::{SourceError, SourceResult};
use crate::http::ExternalFetcher;
use crate::throttle::Throttle;

const MB_BASE: &str = "https://musicbrainz.org/ws/2";
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct MbArtist {
    pub mb_id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct MbRelease {
    pub mb_id: String,
    pub title: String,
    pub year: Option<i16>,
    pub release_type: Option<String>,
    pub primary_artist: Option<MbArtist>,
}

#[derive(Debug, Clone)]
pub struct MbRecording {
    pub primary_artist: Option<MbArtist>,
    pub featured: Vec<MbArtist>,
    pub release: Option<MbRelease>,
    pub score: u32,
}

#[derive(Debug, Clone)]
pub struct MbArtistDetails {
    pub name: Option<String>,
    pub country: Option<String>,
    pub disambiguation: Option<String>,
    pub urls: Vec<MbArtistUrl>,
}

#[derive(Debug, Clone)]
pub struct MbArtistUrl {
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct MbRecordingBrief {
    pub mb_id: String,
    pub title: String,
    pub length_ms: Option<i32>,
    pub first_release_year: Option<i16>,
    pub isrc: Option<String>,
    pub primary_artist: Option<MbArtist>,
    pub featured: Vec<MbArtist>,
    pub release: Option<MbReleaseBrief>,
}

#[derive(Debug, Clone)]
pub struct MbReleaseBrief {
    pub mb_id: String,
    pub title: String,
    pub year: Option<i16>,
    pub release_type: Option<String>,
}

pub struct MbClient {
    fetcher: Arc<ExternalFetcher>,
    throttle: Arc<Throttle>,
}

impl MbClient {
    pub fn new(fetcher: Arc<ExternalFetcher>, rate_limit_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            fetcher,
            throttle: Throttle::new(Duration::from_millis(rate_limit_ms.max(1100))),
        })
    }

    async fn fetch<T: for<'de> Deserialize<'de>>(&self, url: &str) -> SourceResult<Option<T>> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        let bytes = match self.fetcher.get_api(url, headers, &self.throttle).await {
            Ok(bytes) => bytes,
            Err(error) if error.status_code() == Some(404) => return Ok(None),
            Err(error) => return Err(error),
        };
        serde_json::from_slice::<T>(&bytes)
            .map(Some)
            .map_err(|error| SourceError::Invalid(format!("musicbrainz payload: {error}")))
    }

    pub async fn lookup_by_isrc(&self, isrc: &str) -> SourceResult<Option<MbRecording>> {
        let url = format!(
            "{MB_BASE}/isrc/{}?fmt=json&inc=artist-credits+releases+release-groups",
            urlencoding::encode(isrc)
        );
        let body: Option<IsrcResponse> = self.fetch(&url).await?;
        let Some(body) = body else {
            return Ok(None);
        };
        Ok(body
            .recordings
            .into_iter()
            .next()
            .map(|r| recording_from_payload(r, 100)))
    }

    pub async fn lookup_artist(&self, mb_id: &str) -> SourceResult<Option<MbArtistDetails>> {
        let url = format!(
            "{MB_BASE}/artist/{}?inc=url-rels&fmt=json",
            urlencoding::encode(mb_id)
        );
        let body: Option<ArtistPayload> = self.fetch(&url).await?;
        Ok(body.map(|p| MbArtistDetails {
            name: p
                .name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty()),
            country: p.country,
            disambiguation: p.disambiguation.filter(|s| !s.is_empty()),
            urls: p
                .relations
                .unwrap_or_default()
                .into_iter()
                .filter_map(|r| {
                    let kind = r.type_field?;
                    let resource = r.url.and_then(|u| u.resource)?;
                    if resource.is_empty() {
                        None
                    } else {
                        Some(MbArtistUrl {
                            kind,
                            url: resource,
                        })
                    }
                })
                .collect(),
        }))
    }

    pub async fn browse_recordings_by_artist(
        &self,
        mb_id: &str,
        offset: u32,
        limit: u32,
    ) -> SourceResult<Vec<MbRecordingBrief>> {
        let limit = limit.clamp(1, 100);
        let url = format!(
            "{MB_BASE}/recording?artist={}&inc=artist-credits+isrcs+releases+release-groups&fmt=json&limit={limit}&offset={offset}",
            urlencoding::encode(mb_id)
        );
        let body: Option<BrowseResponse> = self.fetch(&url).await?;
        let Some(body) = body else {
            return Ok(Vec::new());
        };
        Ok(body
            .recordings
            .into_iter()
            .map(|r| {
                let credits: Vec<RawCredit> = r.artist_credit.unwrap_or_default();
                let mut artists: Vec<MbArtist> = credits
                    .into_iter()
                    .filter_map(|c| {
                        c.artist.map(|a| MbArtist {
                            mb_id: a.id,
                            name: a.name,
                        })
                    })
                    .collect();
                let primary = if artists.is_empty() {
                    None
                } else {
                    Some(artists.remove(0))
                };
                let isrc = r
                    .isrcs
                    .unwrap_or_default()
                    .into_iter()
                    .find(|s| !s.is_empty());
                let year = r
                    .first_release_date
                    .as_deref()
                    .and_then(|s| s.split('-').next())
                    .and_then(|y| y.parse::<i16>().ok());
                let release = pick_best_release(r.releases.unwrap_or_default()).map(|rel| {
                    let rel_year = rel
                        .date
                        .as_deref()
                        .and_then(|s| s.split('-').next())
                        .and_then(|y| y.parse::<i16>().ok());
                    MbReleaseBrief {
                        mb_id: rel.id,
                        title: rel.title,
                        year: rel_year,
                        release_type: rel.release_group.and_then(|rg| rg.primary_type),
                    }
                });
                MbRecordingBrief {
                    mb_id: r.id,
                    title: r.title,
                    length_ms: r.length,
                    first_release_year: year,
                    isrc,
                    primary_artist: primary,
                    featured: artists,
                    release,
                }
            })
            .collect())
    }

    pub async fn search_recording(
        &self,
        artist: &str,
        title: &str,
        duration_ms: Option<i32>,
    ) -> SourceResult<Option<MbRecording>> {
        let mut q = format!(
            "artist:\"{}\" AND recording:\"{}\"",
            mb_escape(artist),
            mb_escape(title)
        );
        if let Some(ms) = duration_ms {
            let secs = ms / 1000;
            q.push_str(&format!(
                " AND dur:[{} TO {}]",
                (secs - 5).max(0) * 1000,
                (secs + 5) * 1000
            ));
        }
        let url = format!(
            "{MB_BASE}/recording/?query={}&fmt=json&limit=5&inc=artist-credits+releases",
            urlencoding::encode(&q)
        );
        let body: Option<SearchResponse> = self.fetch(&url).await?;
        let Some(body) = body else {
            return Ok(None);
        };
        let best = body
            .recordings
            .into_iter()
            .filter(|r| r.score.unwrap_or(0) >= 80)
            .max_by_key(|r| r.score.unwrap_or(0));
        Ok(best.map(|r| {
            let score = r.score.unwrap_or(0);
            recording_from_payload(r, score)
        }))
    }
}

fn pick_best_release(releases: Vec<RawRelease>) -> Option<RawRelease> {
    let mut scored: Vec<(i32, RawRelease)> = releases
        .into_iter()
        .map(|r| {
            let t = r
                .release_group
                .as_ref()
                .and_then(|g| g.primary_type.as_deref());
            let s = match t {
                Some("Album") => 100,
                Some("Soundtrack") => 80,
                Some("EP") => 70,
                Some("Single") => 60,
                Some("Compilation") => -10,
                Some("Broadcast") => 0,
                Some("Other") => 5,
                None => 5,
                Some(_) => 10,
            };
            (s, r)
        })
        .collect();
    scored.sort_by_key(|(s, _)| -s);
    scored.into_iter().next().map(|(_, r)| r)
}

fn mb_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '"' | '\\'
                | '+'
                | '-'
                | '!'
                | '('
                | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '^'
                | '~'
                | '*'
                | '?'
                | ':'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn recording_from_payload(r: RecordingPayload, score: u32) -> MbRecording {
    let credits: Vec<RawCredit> = r.artist_credit.unwrap_or_default();
    let mut artists: Vec<MbArtist> = credits
        .iter()
        .filter_map(|c| {
            c.artist.as_ref().map(|a| MbArtist {
                mb_id: a.id.clone(),
                name: a.name.clone(),
            })
        })
        .collect();
    let primary = if artists.is_empty() {
        None
    } else {
        Some(artists.remove(0))
    };
    let release = pick_best_release(r.releases.unwrap_or_default()).map(|rel| {
        let year = rel
            .date
            .as_deref()
            .and_then(|s| s.split('-').next())
            .and_then(|y| y.parse::<i16>().ok());
        let release_credits: Vec<RawCredit> = rel.artist_credit.unwrap_or_default();
        let release_primary = release_credits.into_iter().next().and_then(|c| {
            c.artist.map(|a| MbArtist {
                mb_id: a.id,
                name: a.name,
            })
        });
        MbRelease {
            mb_id: rel.id,
            title: rel.title,
            year,
            release_type: rel.release_group.and_then(|rg| rg.primary_type),
            primary_artist: release_primary,
        }
    });
    MbRecording {
        primary_artist: primary,
        featured: artists,
        release,
        score,
    }
}

#[derive(Debug, Deserialize)]
struct IsrcResponse {
    #[serde(default)]
    recordings: Vec<RecordingPayload>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    recordings: Vec<RecordingPayload>,
}

#[derive(Debug, Deserialize)]
struct RecordingPayload {
    #[serde(default)]
    score: Option<u32>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Option<Vec<RawCredit>>,
    #[serde(default)]
    releases: Option<Vec<RawRelease>>,
}

#[derive(Debug, Deserialize)]
struct BrowseResponse {
    #[serde(default)]
    recordings: Vec<BrowseRecording>,
}

#[derive(Debug, Deserialize)]
struct BrowseRecording {
    id: String,
    title: String,
    #[serde(default)]
    length: Option<i32>,
    #[serde(rename = "first-release-date", default)]
    first_release_date: Option<String>,
    #[serde(default)]
    isrcs: Option<Vec<String>>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Option<Vec<RawCredit>>,
    #[serde(default)]
    releases: Option<Vec<RawRelease>>,
}

#[derive(Debug, Deserialize)]
struct RawCredit {
    #[serde(default)]
    artist: Option<RawArtist>,
}

#[derive(Debug, Deserialize)]
struct RawArtist {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawRelease {
    id: String,
    title: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Option<Vec<RawCredit>>,
    #[serde(rename = "release-group", default)]
    release_group: Option<RawReleaseGroup>,
}

#[derive(Debug, Deserialize)]
struct RawReleaseGroup {
    #[serde(rename = "primary-type", default)]
    primary_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtistPayload {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    disambiguation: Option<String>,
    #[serde(default)]
    relations: Option<Vec<ArtistRelation>>,
}

#[derive(Debug, Deserialize)]
struct ArtistRelation {
    #[serde(rename = "type", default)]
    type_field: Option<String>,
    #[serde(default)]
    url: Option<ArtistRelationUrl>,
}

#[derive(Debug, Deserialize)]
struct ArtistRelationUrl {
    #[serde(default)]
    resource: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use bytes::Bytes;
    use call_relay::Request as RelayRequest;

    use crate::http::{RelayFuture, RelayReply, RelayTransport};

    use super::*;

    enum RelayMode {
        Artist,
        Failure,
    }

    struct MockRelay {
        mode: RelayMode,
        headers: Mutex<Option<HashMap<String, String>>>,
    }

    impl MockRelay {
        fn new(mode: RelayMode) -> Arc<Self> {
            Arc::new(Self {
                mode,
                headers: Mutex::new(None),
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
            Box::pin(async { Err(SourceError::NotConfigured("musicbrainz method")) })
        }

        fn fetch(&self, request: RelayRequest) -> RelayFuture<'_, RelayReply> {
            if let Ok(mut headers) = self.headers.lock() {
                *headers = Some(request.headers);
            }
            Box::pin(async move {
                match self.mode {
                    RelayMode::Artist => Ok(RelayReply {
                        status: 200,
                        headers: HashMap::new(),
                        body: Bytes::from_static(
                            br#"{"name":"Artist","country":"US","relations":[]}"#,
                        ),
                    }),
                    RelayMode::Failure => Err(SourceError::Unreachable(
                        "musicbrainz unavailable".to_owned(),
                    )),
                }
            })
        }
    }

    fn client(relay: Arc<MockRelay>) -> Arc<MbClient> {
        let http = sc_fingerprint::builder(None)
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let fetcher = ExternalFetcher::new_with_transport(http, String::new(), relay);
        MbClient::new(fetcher, 1100)
    }

    #[tokio::test]
    async fn requests_use_a_browser_identity_without_configuration() {
        let relay = MockRelay::new(RelayMode::Artist);
        let details = client(relay.clone())
            .lookup_artist("artist-id")
            .await
            .unwrap();

        assert_eq!(
            details.and_then(|details| details.name).as_deref(),
            Some("Artist")
        );
        let headers = relay.headers.lock().unwrap();
        let headers = headers.as_ref().unwrap();
        assert_eq!(
            headers.get("user-agent").map(String::as_str),
            Some(BROWSER_USER_AGENT)
        );
        assert_eq!(
            headers.get("accept-language").map(String::as_str),
            Some("en-US,en;q=0.9")
        );
    }

    #[tokio::test]
    async fn transport_failure_is_not_a_semantic_miss() {
        let relay = MockRelay::new(RelayMode::Failure);
        let result = client(relay).lookup_artist("artist-id").await;

        assert!(matches!(result, Err(SourceError::Unreachable(_))));
    }
}
