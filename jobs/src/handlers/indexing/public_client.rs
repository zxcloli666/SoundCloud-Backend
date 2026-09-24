use std::time::Duration;

use base64::Engine;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use tokio::time::Instant;
use wreq::header::{ACCEPT, ACCEPT_ENCODING, RETRY_AFTER, USER_AGENT};
use wreq::{Client, Response, StatusCode, Url};

use crate::config::DurationConfig;

const CLIENT_ID_TTL: Duration = Duration::from_secs(30 * 60);
const HOME_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const TRACK_RESPONSE_LIMIT: usize = 256 * 1024;
const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(5 * 60);
const MAX_RATE_LIMIT_DELAY: Duration = Duration::from_secs(24 * 60 * 60);
const REJECTION_REFRESH_COOLDOWN: Duration = Duration::from_secs(30);
const WEB_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/124 Safari/537.36";

pub struct PublicSoundCloudClient {
    http: Client,
    api_v2_url: Url,
    web_url: Url,
    proxy_url: Option<Url>,
    proxy_fallback: bool,
    client_id: RwLock<Option<CachedClientId>>,
    client_id_refresh: Mutex<ClientIdRefreshState>,
    pacer: RequestPacer,
}

#[derive(Clone)]
struct CachedClientId {
    value: String,
    loaded_at: Instant,
    generation: u64,
    rejection_refreshed_at: Option<Instant>,
}

#[derive(Default)]
struct ClientIdRefreshState {
    failure: Option<ClientIdRefreshFailure>,
}

struct ClientIdRefreshFailure {
    observed_generation: Option<u64>,
    failed_at: Instant,
}

#[derive(Clone, Copy)]
enum ReadChannel {
    Direct,
    Proxy,
}

#[derive(Debug, thiserror::Error)]
pub enum PublicReadError {
    #[error("SoundCloud track does not exist")]
    NotFound,
    #[error("SoundCloud rate limited public reads for {0:?}")]
    RateLimited(Duration),
    #[error("SoundCloud returned HTTP {0}")]
    Rejected(StatusCode),
    #[error("SoundCloud response exceeded {0} bytes")]
    ResponseTooLarge(usize),
    #[error("SoundCloud public client id is missing")]
    ClientIdMissing,
    #[error("SoundCloud public client id refresh is cooling down for {0:?}")]
    ClientIdRefreshCoolingDown(Duration),
    #[error("SoundCloud track response has no usable duration")]
    DurationMissing,
    #[error("SoundCloud returned invalid JSON: {0}")]
    InvalidJson(#[source] serde_json::Error),
    #[error("SoundCloud request failed: {0}")]
    Transport(#[from] wreq::Error),
}

impl PublicSoundCloudClient {
    pub fn new(config: &DurationConfig) -> Result<Self, wreq::Error> {
        Ok(Self {
            http: sc_fingerprint::builder(None)
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .tcp_keepalive(Duration::from_secs(60))
                .pool_max_idle_per_host(16)
                .redirect(wreq::redirect::Policy::none())
                .build()?,
            api_v2_url: config.api_v2_url.clone(),
            web_url: config.web_url.clone(),
            proxy_url: config.proxy_url.clone(),
            proxy_fallback: config.proxy_fallback,
            client_id: RwLock::new(None),
            client_id_refresh: Mutex::new(ClientIdRefreshState::default()),
            pacer: RequestPacer::new(config.request_gap),
        })
    }

    pub async fn track(&self, track_id: &str) -> Result<i32, PublicReadError> {
        let client_id = self.client_id().await?;
        match self.fetch_track(track_id, &client_id.value).await {
            Err(PublicReadError::Rejected(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)) => {
                let client_id = self.refresh_client_id(Some(client_id.generation)).await?;
                self.fetch_track(track_id, &client_id.value).await
            }
            result => result,
        }
    }

    async fn fetch_track(&self, track_id: &str, client_id: &str) -> Result<i32, PublicReadError> {
        if self.proxy_url.is_none() {
            return self
                .fetch_track_from(ReadChannel::Direct, track_id, client_id)
                .await;
        }
        if !self.proxy_fallback {
            return self
                .fetch_track_from(ReadChannel::Proxy, track_id, client_id)
                .await;
        }

        match self
            .fetch_track_from(ReadChannel::Direct, track_id, client_id)
            .await
        {
            Ok(track) => Ok(track),
            Err(primary) => self
                .fetch_track_from(ReadChannel::Proxy, track_id, client_id)
                .await
                .map_err(|fallback| preferred_error(primary, fallback)),
        }
    }

    async fn fetch_track_from(
        &self,
        channel: ReadChannel,
        track_id: &str,
        client_id: &str,
    ) -> Result<i32, PublicReadError> {
        let mut target = append_path(self.api_v2_url.clone(), &["tracks", track_id])?;
        target.query_pairs_mut().append_pair("client_id", client_id);
        let response = self.send(channel, target).await?;
        if let Some(delay) = response_retry_delay(&response) {
            return Err(PublicReadError::RateLimited(delay));
        }
        match response.status() {
            StatusCode::OK => {
                let body = read_limited(response, TRACK_RESPONSE_LIMIT).await?;
                let track = serde_json::from_slice(&body).map_err(PublicReadError::InvalidJson)?;
                duration_millis(&track).ok_or(PublicReadError::DurationMissing)
            }
            StatusCode::NOT_FOUND | StatusCode::GONE => Err(PublicReadError::NotFound),
            status => Err(PublicReadError::Rejected(status)),
        }
    }

    async fn client_id(&self) -> Result<CachedClientId, PublicReadError> {
        if let Some(cached) = self.client_id.read().await.as_ref()
            && cached.loaded_at.elapsed() < CLIENT_ID_TTL
        {
            return Ok(cached.clone());
        }
        let observed_generation = self
            .client_id
            .read()
            .await
            .as_ref()
            .map(|cached| cached.generation);
        self.refresh_client_id(observed_generation).await
    }

    async fn refresh_client_id(
        &self,
        observed_generation: Option<u64>,
    ) -> Result<CachedClientId, PublicReadError> {
        let mut refresh = self.client_id_refresh.lock().await;
        if let Some(cached) = self.client_id.read().await.as_ref()
            && cached.loaded_at.elapsed() < CLIENT_ID_TTL
            && (observed_generation != Some(cached.generation)
                || cached
                    .rejection_refreshed_at
                    .is_some_and(|at| at.elapsed() < REJECTION_REFRESH_COOLDOWN))
        {
            return Ok(cached.clone());
        }

        if let Some(failure) = refresh.failure.as_ref()
            && failure.observed_generation == observed_generation
            && failure.failed_at.elapsed() < REJECTION_REFRESH_COOLDOWN
        {
            return Err(PublicReadError::ClientIdRefreshCoolingDown(
                REJECTION_REFRESH_COOLDOWN.saturating_sub(failure.failed_at.elapsed()),
            ));
        }

        let client_id = match self.load_client_id().await {
            Ok(client_id) => client_id,
            Err(error) => {
                refresh.failure = Some(ClientIdRefreshFailure {
                    observed_generation,
                    failed_at: Instant::now(),
                });
                return Err(error);
            }
        };
        refresh.failure = None;
        let generation = self
            .client_id
            .read()
            .await
            .as_ref()
            .map_or(1, |cached| cached.generation.wrapping_add(1));
        let cached = CachedClientId {
            value: client_id,
            loaded_at: Instant::now(),
            generation,
            rejection_refreshed_at: observed_generation.map(|_| Instant::now()),
        };
        *self.client_id.write().await = Some(cached.clone());
        Ok(cached)
    }

    async fn load_client_id(&self) -> Result<String, PublicReadError> {
        if self.proxy_url.is_none() {
            return self.load_client_id_from(ReadChannel::Direct).await;
        }
        if !self.proxy_fallback {
            return self.load_client_id_from(ReadChannel::Proxy).await;
        }

        match self.load_client_id_from(ReadChannel::Direct).await {
            Ok(client_id) => Ok(client_id),
            Err(primary) => self
                .load_client_id_from(ReadChannel::Proxy)
                .await
                .map_err(|fallback| preferred_error(primary, fallback)),
        }
    }

    async fn load_client_id_from(&self, channel: ReadChannel) -> Result<String, PublicReadError> {
        let response = self.send(channel, self.web_url.clone()).await?;
        if let Some(delay) = response_retry_delay(&response) {
            return Err(PublicReadError::RateLimited(delay));
        }
        if !response.status().is_success() {
            return Err(PublicReadError::Rejected(response.status()));
        }
        let body = read_limited(response, HOME_RESPONSE_LIMIT).await?;
        extract_client_id(&String::from_utf8_lossy(&body)).ok_or(PublicReadError::ClientIdMissing)
    }

    async fn send(&self, channel: ReadChannel, target: Url) -> Result<Response, PublicReadError> {
        match channel {
            ReadChannel::Direct => self.send_direct(target).await,
            ReadChannel::Proxy => self.send_proxy(target).await,
        }
    }

    async fn send_direct(&self, target: Url) -> Result<Response, PublicReadError> {
        self.pacer.wait().await;
        Ok(self.request(self.http.get(target)).send().await?)
    }

    async fn send_proxy(&self, target: Url) -> Result<Response, PublicReadError> {
        let proxy = self
            .proxy_url
            .clone()
            .ok_or_else(|| PublicReadError::Rejected(StatusCode::INTERNAL_SERVER_ERROR))?;
        self.pacer.wait().await;
        Ok(self
            .request(self.http.get(proxy).header(
                "x-target",
                base64::engine::general_purpose::STANDARD.encode(target.as_str()),
            ))
            .send()
            .await?)
    }

    fn request(&self, request: wreq::RequestBuilder) -> wreq::RequestBuilder {
        request
            .header(ACCEPT, "application/json, text/html;q=0.9")
            .header(ACCEPT_ENCODING, "identity")
            .header(USER_AGENT, WEB_USER_AGENT)
    }
}

struct RequestPacer {
    gap: Duration,
    next_request: Mutex<Instant>,
}

impl RequestPacer {
    fn new(gap: Duration) -> Self {
        Self {
            gap,
            next_request: Mutex::new(Instant::now()),
        }
    }

    async fn wait(&self) {
        let mut next = self.next_request.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        *next = Instant::now() + self.gap;
    }
}

fn append_path(mut base: Url, segments: &[&str]) -> Result<Url, PublicReadError> {
    let mut path = base
        .path_segments_mut()
        .map_err(|_| PublicReadError::Rejected(StatusCode::INTERNAL_SERVER_ERROR))?;
    path.extend(segments);
    drop(path);
    Ok(base)
}

fn response_retry_delay(response: &Response) -> Option<Duration> {
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    match response.status() {
        StatusCode::TOO_MANY_REQUESTS => Some(retry_after.unwrap_or(DEFAULT_RATE_LIMIT_DELAY)),
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => {
            retry_after
        }
        _ => None,
    }
}

fn preferred_error(primary: PublicReadError, fallback: PublicReadError) -> PublicReadError {
    match (primary, fallback) {
        (_, terminal @ PublicReadError::NotFound) => terminal,
        (PublicReadError::RateLimited(left), PublicReadError::RateLimited(right)) => {
            PublicReadError::RateLimited(left.max(right))
        }
        (rate_limited @ PublicReadError::RateLimited(_), _) => rate_limited,
        (_, fallback) => fallback,
    }
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(
            seconds.clamp(1, MAX_RATE_LIMIT_DELAY.as_secs()),
        ));
    }
    let retry_at = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&chrono::Utc);
    let seconds = retry_at
        .signed_duration_since(chrono::Utc::now())
        .num_seconds()
        .max(1);
    u64::try_from(seconds)
        .ok()
        .map(|seconds| Duration::from_secs(seconds.min(MAX_RATE_LIMIT_DELAY.as_secs())))
}

async fn read_limited(response: Response, limit: usize) -> Result<Vec<u8>, PublicReadError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(PublicReadError::ResponseTooLarge(limit));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(PublicReadError::ResponseTooLarge(limit));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn extract_client_id(html: &str) -> Option<String> {
    const MARKER: &str = "\"hydratable\":\"apiClient\"";
    let hydration = &html[html.find(MARKER)? + MARKER.len()..];
    let record_end = hydration.find("\"hydratable\"").unwrap_or(hydration.len());
    let record = &hydration[..record_end];
    let data = &record[record.find("\"data\"")? + 6..];
    let data = &data[data.find(':')? + 1..];
    let value = serde_json::Deserializer::from_str(data)
        .into_iter::<Value>()
        .next()?
        .ok()?;
    let id = value.get("id")?.as_str()?;
    (!id.is_empty() && id.len() <= 128 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| id.to_owned())
}

fn duration_millis(track: &Value) -> Option<i32> {
    let usable = |key| {
        track
            .get(key)
            .and_then(Value::as_i64)
            .filter(|duration| *duration > 0 && *duration != 30_000)
    };
    let duration = usable("full_duration").or_else(|| usable("duration"))?;
    Some(i32::try_from(duration).unwrap_or(i32::MAX))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::extract::{Query, State};
    use axum::http::StatusCode;
    use axum::response::{Html, IntoResponse};
    use axum::routing::get;
    use axum::{Json, Router};

    use super::*;

    #[derive(Clone)]
    struct ServerState {
        home_requests: Arc<AtomicUsize>,
        rotate_client_id: bool,
        reject_tracks: bool,
        reject_home: bool,
    }

    async fn home(State(state): State<ServerState>) -> impl IntoResponse {
        let request = state.home_requests.fetch_add(1, Ordering::Relaxed);
        if state.reject_home {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let client_id = if state.rotate_client_id && request > 0 {
            "fresh"
        } else {
            "stale"
        };
        Html(format!(
            r#"{{"hydratable":"apiClient","data":{{"id":"{client_id}"}}}}"#
        ))
        .into_response()
    }

    async fn track(
        State(state): State<ServerState>,
        Query(query): Query<HashMap<String, String>>,
    ) -> impl IntoResponse {
        if state.reject_tracks || query.get("client_id").map(String::as_str) != Some("fresh") {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Json(serde_json::json!({ "full_duration": 180_000 })).into_response()
    }

    async fn start_server(
        rotate_client_id: bool,
        reject_tracks: bool,
        reject_home: bool,
    ) -> anyhow::Result<(Url, Arc<AtomicUsize>, tokio::task::JoinHandle<()>)> {
        let home_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/", get(home))
            .route("/tracks/{track_id}", get(track))
            .with_state(ServerState {
                home_requests: home_requests.clone(),
                rotate_client_id,
                reject_tracks,
                reject_home,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((format!("http://{address}").parse()?, home_requests, task))
    }

    fn config(url: Url) -> DurationConfig {
        DurationConfig {
            api_v2_url: url.clone(),
            web_url: url,
            proxy_url: None,
            proxy_fallback: false,
            batch_size: 50,
            concurrency: 4,
            request_gap: Duration::from_millis(1),
            max_track_duration_ms: 420_000,
        }
    }

    #[test]
    fn extracts_only_a_non_empty_public_client_id() {
        assert_eq!(
            extract_client_id(r#"{"hydratable":"apiClient","data":{"id":"abc123"}}"#).as_deref(),
            Some("abc123")
        );
        assert!(extract_client_id(r#"{"hydratable":"apiClient","data":{"id":""}}"#).is_none());
    }

    #[test]
    fn api_client_parser_does_not_borrow_another_hydration_id() {
        let html = r#"
            {"hydratable":"apiClient","data":{}}
            {"hydratable":"user","data":{"id":"wrong"}}
        "#;

        assert!(extract_client_id(html).is_none());
    }

    #[test]
    fn full_duration_wins_over_the_preview_sentinel() {
        assert_eq!(
            duration_millis(&serde_json::json!({
                "duration": 30_000,
                "full_duration": 180_000,
            })),
            Some(180_000)
        );
    }

    #[test]
    fn unresolved_sentinels_are_rejected() {
        assert_eq!(
            duration_millis(&serde_json::json!({ "duration": 30_000 })),
            None
        );
        assert_eq!(duration_millis(&serde_json::json!({ "duration": 0 })), None);
        assert_eq!(duration_millis(&serde_json::json!({ "errors": [] })), None);
    }

    #[test]
    fn retry_after_accepts_seconds_and_dates() {
        assert_eq!(parse_retry_after("15"), Some(Duration::from_secs(15)));
        assert_eq!(
            parse_retry_after(&u64::MAX.to_string()),
            Some(MAX_RATE_LIMIT_DELAY)
        );
        assert!(parse_retry_after("invalid").is_none());
    }

    #[tokio::test]
    async fn rejected_client_ids_are_refreshed_once() -> anyhow::Result<()> {
        let (url, _, server) = start_server(true, false, false).await?;
        let client = PublicSoundCloudClient::new(&config(url))?;

        let track = client.track("42").await?;

        server.abort();
        assert_eq!(track, 180_000);
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_requests_share_client_id_refreshes() -> anyhow::Result<()> {
        let (url, home_requests, server) = start_server(true, false, false).await?;
        let client = Arc::new(PublicSoundCloudClient::new(&config(url))?);
        let requests = (0..8).map(|track_id| {
            let client = client.clone();
            async move { client.track(&track_id.to_string()).await }
        });

        let results = futures::future::join_all(requests).await;

        server.abort();
        assert!(results.into_iter().all(|result| result.is_ok()));
        assert_eq!(
            client
                .client_id
                .read()
                .await
                .as_ref()
                .map(|cached| cached.value.as_str()),
            Some("fresh")
        );
        assert_eq!(home_requests.load(Ordering::Relaxed), 2);
        Ok(())
    }

    #[tokio::test]
    async fn same_rejected_client_id_is_not_refreshed_by_every_request() -> anyhow::Result<()> {
        let (url, home_requests, server) = start_server(false, true, false).await?;
        let client = Arc::new(PublicSoundCloudClient::new(&config(url))?);
        let requests = (0..8).map(|track_id| {
            let client = client.clone();
            async move { client.track(&track_id.to_string()).await }
        });

        let results = futures::future::join_all(requests).await;

        server.abort();
        assert!(results.into_iter().all(|result| {
            matches!(
                result,
                Err(PublicReadError::Rejected(StatusCode::UNAUTHORIZED))
            )
        }));
        assert_eq!(home_requests.load(Ordering::Relaxed), 2);
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_homepage_failures_are_negative_cached() -> anyhow::Result<()> {
        let (url, home_requests, server) = start_server(false, false, true).await?;
        let client = Arc::new(PublicSoundCloudClient::new(&config(url))?);
        let requests = (0..8).map(|track_id| {
            let client = client.clone();
            async move { client.track(&track_id.to_string()).await }
        });

        let results = futures::future::join_all(requests).await;

        server.abort();
        assert!(results.into_iter().all(|result| result.is_err()));
        assert_eq!(home_requests.load(Ordering::Relaxed), 1);
        Ok(())
    }
}
