use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use tracing::{info, warn};

use super::free_hq::{FreeSlot, Gate};
use super::quality;
use super::sources::{
    UploadRule, data_response, respond_with_data, try_anon, try_cookies, try_relay_track,
    try_restricted, try_session_oauth,
};
use crate::AppState;
use crate::db::postgres::SessionInfo;
use crate::error::AppError;

pub(crate) const STREAM_DEADLINE: Duration = Duration::from_secs(120);
pub(crate) const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(60);

#[derive(serde::Deserialize)]
pub struct StreamQuery {
    pub ticket: Option<String>,
    pub quality: Option<String>,
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
            AppError::Timeout.into_response()
        }
    }
}

async fn stream_inner(
    State(state): State<AppState>,
    Path(track_urn): Path<String>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Result<Response, AppError> {
    let track_urn = super::storage::canonical_track_urn(&track_urn).ok_or(AppError::NotFound)?;
    let access = extract_stream_access(&state, &track_urn, &query)?;
    let session = state
        .pg
        .get_session(&access.session_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let is_premium = check_is_premium(&state, &session).await;
    let hq = access.high_quality;
    let tier = granted_tier(query.quality.as_deref(), is_premium);
    let secret_token = access.secret_token.as_deref();
    let track = state.pg.find_track(&track_urn).await?;
    let is_public = track.as_ref().map(|track| track.is_public);
    let stored_quality = track.as_ref().and_then(|track| track.stored_quality());
    let kept_quality = track.as_ref().and_then(|track| track.kept_quality());
    let policy = TrackAccessPolicy::new(is_public, secret_token.is_some());

    if state.config.premium_only && !is_premium {
        return Err(AppError::Forbidden);
    }

    if policy.private_only {
        return stream_private(
            &state,
            &access.session_id,
            session.access_token.as_deref(),
            &track_urn,
            secret_token,
            hq,
            is_premium,
        )
        .await;
    }

    let request = StreamRequest {
        state: &state,
        headers: &headers,
        track_urn: &track_urn,
        session_id: &access.session_id,
        access_token: session.access_token.as_deref(),
        secret_token,
        is_premium,
        stored_quality,
        upload: UploadRule {
            cacheable: policy.cacheable,
            kept: kept_quality,
        },
    };
    match tier {
        Some(tier) => stream_tier(&request, tier).await,
        None => stream_any(&request, hq).await,
    }
}

pub(super) fn granted_tier(asked: Option<&str>, is_premium: bool) -> Option<&'static str> {
    let tier = quality::parse(asked?)?;
    if tier == quality::UQ && !is_premium {
        return Some(quality::HQ);
    }
    Some(tier)
}

struct StreamRequest<'a> {
    state: &'a AppState,
    headers: &'a HeaderMap,
    track_urn: &'a str,
    session_id: &'a str,
    access_token: Option<&'a str>,
    secret_token: Option<&'a str>,
    is_premium: bool,
    stored_quality: Option<&'static str>,
    upload: UploadRule,
}

impl StreamRequest<'_> {
    fn uq_live(&self) -> bool {
        self.is_premium && self.state.cookies.as_ref().is_some_and(|c| c.serves_uq())
    }
}

async fn stream_tier(req: &StreamRequest<'_>, tier: &'static str) -> Result<Response, AppError> {
    let tag = "[stream/tier]";
    let StreamRequest {
        state,
        track_urn,
        access_token,
        secret_token,
        is_premium,
        upload,
        ..
    } = *req;

    if tier == quality::UQ {
        if req.uq_live()
            && let Some(r) = try_cookies(state, track_urn, tag, true, is_premium).await
        {
            served(tag, track_urn, "cookies_hq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
    } else if tier == quality::HQ {
        match live_hq(req, tag).await? {
            HqTry::Served(response) => return Ok(response),
            HqTry::Refused(answer) => return Err(answer),
            HqTry::Missed => {}
        }
    } else {
        if let Some(r) = try_relay_track(state, track_urn, quality::SQ).await {
            served(tag, track_urn, "relay_track", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_session_oauth(
            state,
            access_token,
            track_urn,
            secret_token,
            false,
            is_premium,
        )
        .await
        {
            served(tag, track_urn, "oauth_sq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_anon(state, track_urn, tag, quality::SQ).await {
            served(tag, track_urn, "anon", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_cookies(state, track_urn, tag, false, is_premium).await {
            served(tag, track_urn, "cookies_sq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some((r, q)) =
            try_restricted(state, track_urn, tag, quality::SQ, is_premium, upload, None).await
        {
            served(tag, track_urn, "restricted_sq", q);
            return Ok(r);
        }
    }

    info!("{tag} {track_urn} → nothing at {tier}");
    crate::metrics::record_source("none", "exhausted");
    Err(AppError::NoStream)
}

async fn stream_any(req: &StreamRequest<'_>, hq: bool) -> Result<Response, AppError> {
    let tag = "[stream]";
    let StreamRequest {
        state,
        track_urn,
        access_token,
        secret_token,
        is_premium,
        stored_quality,
        upload,
        ..
    } = *req;
    let uq_live = hq && req.uq_live();
    let wanted = match (hq, uq_live) {
        (false, _) => quality::SQ,
        (true, false) => quality::HQ,
        (true, true) => quality::UQ,
    };
    let stored = stored_quality.map_or(0, quality::rank);

    if upload.cacheable
        && stored >= quality::rank(wanted)
        && let Some(response) = from_storage(req, tag).await
    {
        return Ok(response);
    }

    if !hq && let Some(r) = try_relay_track(state, track_urn, quality::SQ).await {
        served(tag, track_urn, "relay_track", r.2);
        return respond_with_data(state, track_urn, r, upload);
    }

    if hq {
        if uq_live && let Some(r) = try_cookies(state, track_urn, tag, true, is_premium).await {
            served(tag, track_urn, "cookies_hq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if upload.cacheable
            && stored >= quality::rank(quality::HQ)
            && stored < quality::rank(wanted)
            && let Some(response) = from_storage(req, tag).await
        {
            return Ok(response);
        }
        if let HqTry::Served(response) = live_hq(req, tag).await? {
            return Ok(response);
        }
        if upload.cacheable
            && stored < quality::rank(quality::HQ)
            && let Some(response) = from_storage(req, tag).await
        {
            return Ok(response);
        }
        if let Some(r) = try_session_oauth(
            state,
            access_token,
            track_urn,
            secret_token,
            false,
            is_premium,
        )
        .await
        {
            served(tag, track_urn, "oauth_sq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_anon(state, track_urn, tag, quality::SQ).await {
            served(tag, track_urn, "anon", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_cookies(state, track_urn, tag, false, is_premium).await {
            served(tag, track_urn, "cookies_sq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_relay_track(state, track_urn, quality::SQ).await {
            served(tag, track_urn, "relay_track", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
    } else {
        if let Some(r) = try_session_oauth(
            state,
            access_token,
            track_urn,
            secret_token,
            false,
            is_premium,
        )
        .await
        {
            served(tag, track_urn, "oauth_sq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_anon(state, track_urn, tag, quality::SQ).await {
            served(tag, track_urn, "anon", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
        if let Some(r) = try_cookies(state, track_urn, tag, false, is_premium).await {
            served(tag, track_urn, "cookies_sq", r.2);
            return respond_with_data(state, track_urn, r, upload);
        }
    }

    if let Some((r, q)) =
        try_restricted(state, track_urn, tag, quality::SQ, is_premium, upload, None).await
    {
        served(tag, track_urn, "restricted_sq", q);
        return Ok(r);
    }

    warn!("{tag} {track_urn} → no stream available");
    crate::metrics::record_source("none", "exhausted");
    Err(AppError::NoStream)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveHq {
    Oauth,
    Restricted,
    Anon,
}

fn live_hq_order(is_premium: bool) -> [LiveHq; 3] {
    if is_premium {
        [LiveHq::Oauth, LiveHq::Restricted, LiveHq::Anon]
    } else {
        [LiveHq::Anon, LiveHq::Restricted, LiveHq::Oauth]
    }
}

enum HqTry {
    Served(Response),
    Missed,
    Refused(AppError),
}

async fn live_hq(req: &StreamRequest<'_>, tag: &str) -> Result<HqTry, AppError> {
    let StreamRequest {
        state,
        track_urn,
        session_id,
        access_token,
        secret_token,
        is_premium,
        upload,
        ..
    } = *req;

    let gate = state.free_hq.gate(track_urn, session_id, is_premium);
    let slot = match free_slot(gate, tag, track_urn, "none") {
        Ok(slot) => slot,
        Err(answer) => return Ok(HqTry::Refused(answer)),
    };

    for source in live_hq_order(is_premium) {
        match source {
            LiveHq::Oauth => {
                if let Some(r) = try_session_oauth(
                    state,
                    access_token,
                    track_urn,
                    secret_token,
                    true,
                    is_premium,
                )
                .await
                {
                    served(tag, track_urn, "oauth_hq", r.2);
                    return respond_with_data(state, track_urn, r, upload).map(HqTry::Served);
                }
            }
            LiveHq::Restricted => {
                if let Some((r, q)) = try_restricted(
                    state,
                    track_urn,
                    tag,
                    quality::HQ,
                    is_premium,
                    upload,
                    slot.clone(),
                )
                .await
                {
                    served(tag, track_urn, "restricted_hq", q);
                    return Ok(HqTry::Served(r));
                }
            }
            LiveHq::Anon => {
                if let Some(r) = try_anon(state, track_urn, tag, quality::HQ).await {
                    served(tag, track_urn, "anon_hq", r.2);
                    return respond_with_data(state, track_urn, r, upload).map(HqTry::Served);
                }
            }
        }
    }
    if !is_premium {
        state.free_hq.note_miss(track_urn);
    }
    Ok(HqTry::Missed)
}

pub(super) fn free_slot(
    gate: Gate,
    tag: &str,
    track_urn: &str,
    source: &'static str,
) -> Result<Option<Arc<FreeSlot>>, AppError> {
    match gate {
        Gate::Open(slot) => Ok(slot),
        Gate::Busy(wait) => {
            info!("{tag} {track_urn} → no free hq slot for this session");
            crate::metrics::record_source(source, "free_hq_busy");
            Err(AppError::FreeHqBusy(wait))
        }
        Gate::MissedLately => {
            info!("{tag} {track_urn} → had no hq lately");
            crate::metrics::record_source(source, "free_hq_missed_lately");
            Err(AppError::NoStream)
        }
    }
}

fn served(tag: &str, track_urn: &str, source: &'static str, quality: &str) {
    info!("{tag} {track_urn} → {source} ({quality})");
    crate::metrics::record_source(source, "served");
}

async fn from_storage(req: &StreamRequest<'_>, tag: &str) -> Option<Response> {
    let StreamRequest {
        state,
        headers,
        track_urn,
        stored_quality,
        ..
    } = *req;
    let shown = stored_quality.unwrap_or("?");
    if headers.contains_key("x-session-id") {
        let response = state.storage.try_proxy(track_urn, stored_quality).await?;
        served(tag, track_urn, "cdn_proxy", shown);
        return Some(response);
    }
    let cdn_url = state.storage.try_serve(track_urn).await?;
    served(tag, track_urn, "cdn_redirect", shown);
    Some(Redirect::temporary(&cdn_url).into_response())
}

async fn stream_private(
    state: &AppState,
    session_id: &str,
    access_token: Option<&str>,
    track_urn: &str,
    secret_token: Option<&str>,
    high_quality: bool,
    is_premium: bool,
) -> Result<Response, AppError> {
    if high_quality
        && let Gate::Open(_slot) = state.free_hq.gate(track_urn, session_id, is_premium)
        && let Some((data, content_type, quality)) = try_session_oauth(
            state,
            access_token,
            track_urn,
            secret_token,
            true,
            is_premium,
        )
        .await
    {
        return data_response(data, content_type, quality);
    }
    if let Some((data, content_type, quality)) = try_session_oauth(
        state,
        access_token,
        track_urn,
        secret_token,
        false,
        is_premium,
    )
    .await
    {
        return data_response(data, content_type, quality);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::free_hq::{BUSY_RETRY_AFTER, FreeHq, SHUT_RETRY_AFTER};
    use axum::http::{HeaderValue, StatusCode, header};

    const SOURCES: [&str; 11] = [
        "cdn_proxy",
        "cdn_redirect",
        "relay_track",
        "oauth_hq",
        "oauth_sq",
        "cookies_hq",
        "cookies_sq",
        "anon",
        "anon_hq",
        "restricted_hq",
        "restricted_sq",
    ];

    fn labels_in_the_cascade() -> Vec<String> {
        let body = crate::source_tree::read("src/stream/handler.rs");
        let mut found = Vec::new();
        let mut rest = body.as_str();
        while let Some(start) = rest.find("served(tag, ") {
            rest = &rest[start + "served(tag, ".len()..];
            let line = rest.lines().next().unwrap_or("");
            let Some((_, label)) = line.split_once('"') else {
                continue;
            };
            let Some((label, _)) = label.split_once('"') else {
                continue;
            };
            found.push(label.to_owned());
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
    fn the_path_is_canonical_before_the_ticket_or_any_storage_key_sees_it() {
        for (file, source, entrance) in [
            (
                "handler.rs",
                include_str!("handler.rs"),
                "async fn stream_inner(",
            ),
            (
                "download.rs",
                include_str!("download.rs"),
                "async fn download_inner(",
            ),
        ] {
            let body = source
                .split_once(entrance)
                .expect("the entrance is still here")
                .1;
            let canonical = body
                .find("canonical_track_urn(&track_urn)")
                .unwrap_or_else(|| panic!("{file} reads its track without canonicalizing it"));
            for later in ["extract_stream_access", "collect_entries", "storage."] {
                if let Some(at) = body.find(later) {
                    assert!(
                        canonical < at,
                        "{file}: `{later}` runs before the path is canonical, so a bare id \
                         could reach a ticket or an object key"
                    );
                }
            }
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
    fn hq_is_open_to_everyone_and_uq_drops_to_hq_without_star() {
        assert_eq!(granted_tier(Some("sq"), false), Some(quality::SQ));
        assert_eq!(granted_tier(Some("hq"), false), Some(quality::HQ));
        assert_eq!(granted_tier(Some("uq"), false), Some(quality::HQ));
        assert_eq!(granted_tier(Some("uq"), true), Some(quality::UQ));
        assert_eq!(granted_tier(Some("hq"), true), Some(quality::HQ));
        assert_eq!(granted_tier(Some("lossless"), true), None);
        assert_eq!(granted_tier(None, false), None);
    }

    #[test]
    fn full_free_slots_ask_to_come_back_later_and_a_track_without_hq_steps_down() {
        const URN: &str = "soundcloud:tracks:1";
        let retry_after = |free: &FreeHq| {
            let busy = free_slot(free.gate(URN, "session", false), "[test]", URN, "none")
                .unwrap_err()
                .into_response();
            assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
            busy.headers().get(header::RETRY_AFTER).unwrap().clone()
        };

        let free = FreeHq::new(1);
        let _held = free.gate(URN, "another", false);
        assert_eq!(
            retry_after(&free),
            BUSY_RETRY_AFTER.as_secs().to_string().as_str(),
            "a 404 here makes clients ask hq again on the very next track"
        );
        let full = FreeHq::new(0);
        assert_eq!(
            retry_after(&full),
            SHUT_RETRY_AFTER.as_secs().to_string().as_str(),
            "with FREE_HQ_FETCHES=0 no slot ever opens; asking every 30 s only costs lookups"
        );

        let free = FreeHq::new(1);
        free.note_miss(URN);
        let missed = free_slot(free.gate(URN, "session", false), "[test]", URN, "none")
            .unwrap_err()
            .into_response();
        assert_eq!(missed.status(), StatusCode::NOT_FOUND);
        assert!(missed.headers().get(header::RETRY_AFTER).is_none());

        assert!(
            free_slot(full.gate(URN, "session", true), "[test]", URN, "none")
                .is_ok_and(|slot| slot.is_none())
        );
    }

    #[test]
    fn a_free_session_reads_public_sources_before_its_own_token() {
        assert_eq!(
            live_hq_order(false),
            [LiveHq::Anon, LiveHq::Restricted, LiveHq::Oauth],
            "most public tracks carry a plain aac_160k and none carry it both plain and encrypted, \
             so anon first skips a second meta read and never pays for a license; free misses are \
             mass reads, apiv1 is the last resort for them"
        );
        assert_eq!(
            live_hq_order(true),
            [LiveHq::Oauth, LiveHq::Restricted, LiveHq::Anon]
        );
    }

    #[test]
    fn only_star_reaches_a_cookies_account() {
        let sources = crate::source_tree::read("src/stream/sources.rs");
        for entrance in ["async fn try_cookies(", "async fn restricted_source("] {
            let body = sources
                .split_once(entrance)
                .unwrap_or_else(|| panic!("{entrance} is gone"))
                .1;
            let body = body.split_once("\n}\n").expect("the function ends").0;
            let gate = body
                .find("if !premium")
                .unwrap_or_else(|| panic!("{entrance} does not check star"));
            let cookies = body
                .find("state.cookies")
                .unwrap_or_else(|| panic!("{entrance} no longer reads cookies"));
            assert!(
                gate < cookies,
                "{entrance} reaches a cookies account before it checks star; those accounts \
                 are few and free traffic would put them on cooldown for everyone"
            );
        }
    }

    #[test]
    fn a_private_track_asks_for_a_free_hq_slot_like_any_other() {
        let source = include_str!("handler.rs");
        let body = source
            .split_once("async fn stream_private(")
            .expect("the private path is still here")
            .1;
        let body = body.split_once("\n}\n").expect("the function ends").0;
        let gate = body
            .find("free_hq.gate(")
            .expect("private hq goes through the free hq gate");
        let hq = body
            .find("try_session_oauth(")
            .expect("private tracks still read the session token");
        assert!(
            gate < hq,
            "a secret link with an hq ticket would reach SoundCloud past the free hq cap, \
             and FREE_HQ_FETCHES=0 would not stop it"
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
