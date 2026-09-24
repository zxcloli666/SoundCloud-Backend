use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use bytes::Bytes;
use futures::StreamExt;
use tracing::{info, warn};

use crate::AppState;
use crate::db::postgres::SessionInfo;
use crate::error::AppError;

pub(crate) const STREAM_DEADLINE: Duration = Duration::from_secs(120);
pub(crate) const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(60);

#[derive(serde::Deserialize)]
pub struct StreamQuery {
    pub ticket: Option<String>,
}

pub(crate) struct StreamAccess {
    pub session_id: String,
    pub secret_token: Option<String>,
    pub high_quality: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TrackAccessPolicy {
    private_only: bool,
    cacheable: bool,
}

impl TrackAccessPolicy {
    fn new(is_public: Option<bool>, has_secret: bool) -> Self {
        Self {
            private_only: has_secret || is_public == Some(false),
            cacheable: !has_secret && is_public == Some(true),
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ResolveQuery {
    pub url: String,
}

pub async fn resolve_track(
    State(state): State<AppState>,
    Query(query): Query<ResolveQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    match state.anon.resolve_url(&query.url).await {
        Ok(track) => Ok(Json(track)),
        Err(error) => {
            warn!("[resolve] {} failed: {error}", query.url);
            Err(AppError::NotFound)
        }
    }
}

pub async fn stream(
    state: State<AppState>,
    track_urn: Path<String>,
    headers: HeaderMap,
    query: Query<StreamQuery>,
) -> Response {
    let urn_for_log = track_urn.0.clone();
    match tokio::time::timeout(
        STREAM_DEADLINE,
        stream_inner(state, track_urn, headers, query),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error.into_response(),
        Err(_) => {
            warn!("[stream] {urn_for_log} → deadline {STREAM_DEADLINE:?} exceeded");
            crate::metrics::record_source("none", "deadline");
            AppError::NoStream.into_response()
        }
    }
}

async fn stream_inner(
    State(state): State<AppState>,
    Path(track_urn): Path<String>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Result<Response, AppError> {
    let access = extract_stream_access(&state, &track_urn, &query)?;
    let session = state
        .pg
        .get_session(&access.session_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let is_premium = check_is_premium(&state, &session).await;
    let hq = access.high_quality;
    let secret_token = access.secret_token.as_deref();
    let is_public = state.pg.track_is_public(&track_urn).await?;
    let policy = TrackAccessPolicy::new(is_public, secret_token.is_some());

    if state.config.premium_only && !is_premium {
        return Err(AppError::Forbidden);
    }
    if hq && !is_premium {
        return Err(AppError::Forbidden);
    }

    if policy.private_only {
        return stream_private(
            &state,
            session.access_token.as_deref(),
            &track_urn,
            secret_token,
            hq,
        )
        .await;
    }

    let tag = "[stream]";
    let cacheable = policy.cacheable;

    if cacheable {
        if headers.contains_key("x-session-id") {
            if let Some(response) = state.storage.try_proxy(&track_urn).await {
                served(tag, &track_urn, "cdn_proxy");
                return Ok(response);
            }
        } else if let Some(cdn_url) = state.storage.try_serve(&track_urn).await {
            served(tag, &track_urn, "cdn_redirect");
            return Ok(Redirect::temporary(&cdn_url).into_response());
        }
    }

    if !hq && let Some(r) = try_relay_track(&state, &track_urn, "sq").await {
        served(tag, &track_urn, "relay_track");
        return respond_with_data(&state, &track_urn, r.0, r.1, "sq", cacheable);
    }

    let access = session.access_token.as_deref();

    if hq {
        if let Some(r) = try_session_oauth(&state, access, &track_urn, secret_token, true).await {
            served(tag, &track_urn, "oauth_hq");
            return respond_with_data(&state, &track_urn, r.0, r.1, "hq", cacheable);
        }
        if let Some(r) = try_cookies(&state, &track_urn, tag, true).await {
            served(tag, &track_urn, "cookies_hq");
            return respond_with_data(&state, &track_urn, r.0, r.1, "hq", cacheable);
        }
        if let Some(r) = try_restricted(&state, &track_urn, tag, true, cacheable).await {
            served(tag, &track_urn, "restricted_hq");
            return Ok(r);
        }
        if let Some(r) = try_session_oauth(&state, access, &track_urn, secret_token, false).await {
            served(tag, &track_urn, "oauth_sq");
            return respond_with_data(&state, &track_urn, r.0, r.1, "sq", cacheable);
        }
        if let Some(r) = try_anon(&state, &track_urn, tag).await {
            served(tag, &track_urn, "anon");
            return respond_with_data(&state, &track_urn, r.0, r.1, "sq", cacheable);
        }
        if let Some(r) = try_cookies(&state, &track_urn, tag, false).await {
            served(tag, &track_urn, "cookies_sq");
            return respond_with_data(&state, &track_urn, r.0, r.1, "sq", cacheable);
        }
    } else {
        if let Some(r) = try_session_oauth(&state, access, &track_urn, secret_token, false).await {
            served(tag, &track_urn, "oauth_sq");
            return respond_with_data(&state, &track_urn, r.0, r.1, "sq", cacheable);
        }
        if let Some(r) = try_anon(&state, &track_urn, tag).await {
            served(tag, &track_urn, "anon");
            return respond_with_data(&state, &track_urn, r.0, r.1, "sq", cacheable);
        }
        if is_premium && let Some(r) = try_cookies(&state, &track_urn, tag, false).await {
            served(tag, &track_urn, "cookies_sq");
            return respond_with_data(&state, &track_urn, r.0, r.1, "sq", cacheable);
        }
    }

    if let Some(r) = try_restricted(&state, &track_urn, tag, false, cacheable).await {
        served(tag, &track_urn, "restricted_sq");
        return Ok(r);
    }

    warn!("{tag} {track_urn} → no stream available");
    crate::metrics::record_source("none", "exhausted");
    Err(AppError::NoStream)
}

fn served(tag: &str, track_urn: &str, source: &'static str) {
    info!("{tag} {track_urn} → {source}");
    crate::metrics::record_source(source, "served");
}

async fn stream_private(
    state: &AppState,
    access_token: Option<&str>,
    track_urn: &str,
    secret_token: Option<&str>,
    high_quality: bool,
) -> Result<Response, AppError> {
    if high_quality
        && let Some((data, content_type)) =
            try_session_oauth(state, access_token, track_urn, secret_token, true).await
    {
        return data_response(data, content_type);
    }
    if let Some((data, content_type)) =
        try_session_oauth(state, access_token, track_urn, secret_token, false).await
    {
        return data_response(data, content_type);
    }

    warn!("[stream] {track_urn} → private stream unavailable");
    Err(AppError::NoStream)
}

pub(crate) async fn check_is_premium(state: &AppState, session: &SessionInfo) -> bool {
    let Some(user) = session.soundcloud_user_id.as_deref() else {
        return false;
    };
    state.pg.is_premium(user).await.unwrap_or(false)
}

async fn try_oauth(
    state: &AppState,
    access_token: &str,
    track_urn: &str,
    secret_token: Option<&str>,
    hq_only: bool,
) -> Option<(Bytes, &'static str)> {
    let ctx = super::oauth::OauthCtx {
        client: &state.http_client,
        pg: &state.pg,
        proxy_url: &state.config.sc_proxy_url,
        proxy_fallback: state.config.sc_proxy_fallback,
        fallback_session_count: state.config.sc_oauth_fallback_sessions,
    };
    let result =
        super::oauth::try_oauth_stream(&ctx, access_token, track_urn, secret_token, hq_only)
            .await?;
    Some((result.data, result.content_type))
}

async fn try_session_oauth(
    state: &AppState,
    access_token: Option<&str>,
    track_urn: &str,
    secret_token: Option<&str>,
    hq_only: bool,
) -> Option<(Bytes, &'static str)> {
    let access_token = access_token?;
    try_oauth(state, access_token, track_urn, secret_token, hq_only).await
}

async fn try_cookies(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    hq_only: bool,
) -> Option<(Bytes, &'static str)> {
    let cookies_client = state.cookies.as_ref()?;
    match cookies_client.get_stream(track_urn, hq_only).await {
        Ok(Some(result)) => Some((result.data, result.content_type)),
        Ok(None) => {
            warn!("{tag} {track_urn} cookies returned nothing");
            None
        }
        Err(e) => {
            warn!("{tag} {track_urn} cookies failed: {e}");
            None
        }
    }
}

async fn try_anon(state: &AppState, track_urn: &str, tag: &str) -> Option<(Bytes, &'static str)> {
    match state.anon.get_stream(track_urn).await {
        Ok(Some(result)) => Some((result.data, result.content_type)),
        Ok(None) => {
            warn!("{tag} {track_urn} anon returned nothing");
            None
        }
        Err(e) => {
            warn!("{tag} {track_urn} anon failed: {e}");
            None
        }
    }
}

async fn restricted_source(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    hq_first: bool,
) -> Option<crate::stream::restricted::RestrictedSource> {
    match state.anon.resolve_restricted(track_urn, hq_first).await {
        Ok(Some(v)) => return Some(v),
        Ok(None) => {}
        Err(e) => warn!("{tag} {track_urn} restricted(anon) failed: {e}"),
    }
    let cookies = state.cookies.as_ref()?;
    match cookies.resolve_restricted(track_urn, hq_first).await {
        Ok(Some(v)) => Some(v),
        Ok(None) => None,
        Err(e) => {
            warn!("{tag} {track_urn} restricted(cookies) failed: {e}");
            None
        }
    }
}

async fn try_relay_track(
    state: &AppState,
    track_urn: &str,
    quality: &str,
) -> Option<(Bytes, &'static str)> {
    let id = track_urn.rsplit(':').next()?;
    if id.is_empty() || id == track_urn {
        return None; // not a canonical soundcloud:tracks:<id> urn
    }
    let client_id = state.anon.get_client_id().await.unwrap_or_default();
    let (audio, ct) = crate::stream::proxy::get_track_via_relay(
        id,
        quality,
        &client_id,
        state.config.edge_wvd_url.as_deref(),
        state.config.edge_wvd_token.as_deref(),
    )
    .await?;
    Some((
        Bytes::from(audio),
        crate::stream::hls::mime_to_content_type(&ct),
    ))
}

async fn try_restricted(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    hq_first: bool,
    cacheable: bool,
) -> Option<Response> {
    let src = restricted_source(state, track_urn, tag, hq_first).await?;

    if let (Some(wvd_url), Some(wvd_token)) = (
        state.config.edge_wvd_url.as_deref(),
        state.config.edge_wvd_token.as_deref(),
    ) && let Some(audio) =
        crate::stream::proxy::hls_decrypt_via_relay(&src.manifest, &src.token, wvd_url, wvd_token)
            .await
    {
        let quality = if src.is_hq { "hq" } else { "sq" };
        let bytes = Bytes::from(audio);
        if cacheable && bytes.len() > 8192 {
            state.storage.upload_in_background_with_quality(
                track_urn.to_string(),
                bytes.clone(),
                quality,
            );
        }
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", src.content_type)
                .body(Body::from(bytes))
                .unwrap(),
        );
    }

    let engine = state.decryptor.as_ref()?;
    let fetcher: std::sync::Arc<dyn decrypt::Fetcher> =
        std::sync::Arc::new(crate::stream::decrypt_fetch::ProxyFetcher {
            client: state.http_client.clone(),
            proxy_url: state.config.sc_proxy_url.clone(),
        });
    let stream = match engine
        .process_stream(&src.manifest, &src.token, fetcher)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!("{tag} {track_urn} restricted decode failed: {e}");
            return None;
        }
    };

    if !cacheable {
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", src.content_type)
                .body(Body::from_stream(stream))
                .unwrap(),
        );
    }

    let acc = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let acc_w = acc.clone();
    let storage = state.storage.clone();
    let urn = track_urn.to_string();
    let quality = if src.is_hq { "hq" } else { "sq" };
    let teed = stream
        .map(move |chunk| {
            if let Ok(b) = &chunk {
                acc_w.lock().unwrap().extend_from_slice(b);
            }
            chunk
        })
        .chain(futures::stream::once(async move {
            let data = std::mem::take(&mut *acc.lock().unwrap());
            if data.len() > 8192 {
                storage.upload_in_background_with_quality(urn, Bytes::from(data), quality);
            }
            Ok::<_, decrypt::Error>(Bytes::new())
        }));

    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", src.content_type)
            .body(Body::from_stream(teed))
            .unwrap(),
    )
}

fn extract_header_session_id(headers: &HeaderMap) -> Result<String, AppError> {
    if let Some(val) = headers.get("x-session-id") {
        return val
            .to_str()
            .map(|s| s.to_string())
            .map_err(|_| AppError::Unauthorized);
    }
    Err(AppError::Unauthorized)
}

pub(crate) fn extract_download_session_id(headers: &HeaderMap) -> Result<String, AppError> {
    extract_header_session_id(headers)
}

pub(crate) fn extract_stream_access(
    state: &AppState,
    track_urn: &str,
    query: &StreamQuery,
) -> Result<StreamAccess, AppError> {
    let ticket = query.ticket.as_deref().ok_or(AppError::Unauthorized)?;
    let ticket = state
        .config
        .stream_ticket_keys
        .verify(ticket, track_urn)
        .map_err(|_| AppError::Unauthorized)?;
    Ok(StreamAccess {
        session_id: ticket.session_id.to_string(),
        secret_token: ticket.secret_token,
        high_quality: ticket.high_quality,
    })
}

fn respond_with_data(
    state: &AppState,
    track_urn: &str,
    data: Bytes,
    content_type: &'static str,
    quality: &'static str,
    cacheable: bool,
) -> Result<Response, AppError> {
    if cacheable && data.len() > 8192 {
        state.storage.upload_in_background_with_quality(
            track_urn.to_string(),
            data.clone(),
            quality,
        );
    }

    data_response(data, content_type)
}

fn data_response(data: Bytes, content_type: &'static str) -> Result<Response, AppError> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .header("content-length", data.len().to_string())
        .body(Body::from(data))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const SOURCES: [&str; 10] = [
        "cdn_proxy",
        "cdn_redirect",
        "relay_track",
        "oauth_hq",
        "oauth_sq",
        "cookies_hq",
        "cookies_sq",
        "anon",
        "restricted_hq",
        "restricted_sq",
    ];

    fn labels_in_the_cascade() -> Vec<String> {
        let body = crate::source_tree::read("src/stream/handler.rs");
        let mut found = Vec::new();
        let mut rest = body.as_str();
        while let Some(start) = rest.find("served(tag, &track_urn, \"") {
            rest = &rest[start + "served(tag, &track_urn, \"".len()..];
            let Some(end) = rest.find('"') else { break };
            found.push(rest[..end].to_owned());
            rest = &rest[end..];
        }
        found
    }

    #[test]
    fn the_cascade_can_only_report_a_source_that_was_declared() {
        let used = labels_in_the_cascade();
        assert!(
            used.len() >= SOURCES.len(),
            "the scan found {} branches for {} declared sources; it is reading the wrong text",
            used.len(),
            SOURCES.len()
        );
        for label in &used {
            assert!(
                SOURCES.contains(&label.as_str()),
                "the cascade reports `{label}`, which is not a declared source; every label \
                 becomes its own metric series, so a typo here mints a series nobody can find"
            );
        }
    }

    #[test]
    fn no_source_is_declared_that_nothing_can_reach() {
        let used = labels_in_the_cascade();
        for source in SOURCES {
            assert!(
                used.iter().any(|label| label == source),
                "`{source}` is declared but no branch reports it; a dashboard would wait \
                 forever for a series that never arrives"
            );
        }
    }

    #[test]
    fn a_stream_is_opened_by_a_ticket_and_by_nothing_else() {
        let source = include_str!("handler.rs");
        let opened = source
            .split_once("fn extract_stream_access(")
            .expect("the stream entrance is still here")
            .1;
        let opened = opened.split_once("\n}\n").expect("the function ends").0;

        assert!(
            opened.contains("stream_ticket_keys"),
            "the ticket is what binds the request to one track for two minutes"
        );
        for taken_from_the_caller in ["headers", "query.secret_token", "query.hq", "HeaderMap"] {
            assert!(
                !opened.contains(taken_from_the_caller),
                "the api checks local access — deleted, private, premium — before it issues a \
                 ticket; reading `{taken_from_the_caller}` here lets a caller reach the audio \
                 while skipping that check entirely"
            );
        }
    }

    #[test]
    fn a_download_still_answers_the_session_header_because_nothing_issues_it_a_ticket() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-id", HeaderValue::from_static("header-session"));

        assert_eq!(
            extract_header_session_id(&headers).unwrap(),
            "header-session"
        );
        assert!(extract_header_session_id(&HeaderMap::new()).is_err());
    }

    #[test]
    fn download_session_comes_only_from_header() {
        assert!(extract_download_session_id(&HeaderMap::new()).is_err());
        let mut headers = HeaderMap::new();
        headers.insert("x-session-id", HeaderValue::from_static("header-session"));
        assert_eq!(
            extract_download_session_id(&headers).unwrap(),
            "header-session"
        );
    }

    #[test]
    fn only_known_public_tracks_can_use_shared_storage() {
        assert_eq!(
            TrackAccessPolicy::new(Some(true), false),
            TrackAccessPolicy {
                private_only: false,
                cacheable: true,
            }
        );
        for is_public in [Some(false), None] {
            assert!(!TrackAccessPolicy::new(is_public, false).cacheable);
        }
        let secret = TrackAccessPolicy::new(Some(true), true);
        assert!(secret.private_only);
        assert!(!secret.cacheable);
    }
}
