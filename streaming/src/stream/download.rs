use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use base64::Engine as _;
use bytes::Bytes;
use futures::future::BoxFuture;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, info, warn};
use wreq::Client;

use super::free_hq::{FreeHq, FreeSlot};
use super::handler::{
    DOWNLOAD_DEADLINE, check_is_premium, extract_download_session_id, free_slot, granted_tier,
};
use super::proxy::{fetch_get_bytes, fetch_get_json, fetch_get_text};
use super::quality;
use super::restricted::{Transcoding, build_transcoding_target};
use crate::AppState;
use crate::error::AppError;

#[derive(Serialize)]
pub struct DownloadResponse {
    pub track_urn: String,
    pub candidates: Vec<Candidate>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Candidate {
    Progressive {
        quality: String,
        preset: String,
        mime: String,
        url: String,
    },
    Hls {
        quality: String,
        preset: String,
        mime: String,
        manifest_url: String,
    },
    EncryptedHls {
        quality: String,
        preset: String,
        mime: String,
        content_type: String,
        init_base64: String,
        segments: Vec<String>,
        key_base64: String,
    },
}

struct Entry {
    t: Transcoding,
    client_id: String,
    track_auth: Option<String>,
    headers: HashMap<String, String>,
}

#[derive(serde::Deserialize)]
pub struct DownloadQuery {
    pub quality: Option<String>,
}

#[derive(serde::Deserialize)]
struct ResolveResp {
    url: String,
    #[serde(rename = "licenseAuthToken")]
    auth_token: Option<String>,
}

pub async fn download(
    state: State<AppState>,
    track_urn: Path<String>,
    headers: HeaderMap,
    query: Query<DownloadQuery>,
) -> Result<Json<DownloadResponse>, AppError> {
    let urn_for_log = track_urn.0.clone();
    let inner = download_inner(state, track_urn, headers, query);
    match tokio::time::timeout(DOWNLOAD_DEADLINE, inner).await {
        Ok(r) => r,
        Err(_) => {
            warn!("[download] {urn_for_log} → deadline {DOWNLOAD_DEADLINE:?} exceeded");
            Err(AppError::Timeout)
        }
    }
}

async fn download_inner(
    State(state): State<AppState>,
    Path(track_urn): Path<String>,
    headers: HeaderMap,
    Query(query): Query<DownloadQuery>,
) -> Result<Json<DownloadResponse>, AppError> {
    let track_urn = super::storage::canonical_track_urn(&track_urn).ok_or(AppError::NotFound)?;
    let session_id = extract_download_session_id(&headers)?;
    let session = state
        .pg
        .get_session(&session_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let is_premium = check_is_premium(&state, &session).await;

    if state.config.premium_only && !is_premium {
        return Err(AppError::Forbidden);
    }

    let (entries, _slot) = if is_premium {
        (collect_entries(&state, &track_urn, true).await, None)
    } else {
        let tier = free_tier(query.quality.as_deref());
        free_tier_entries(&state, &track_urn, &session_id, tier).await?
    };
    if entries.is_empty() {
        warn!("[download] {track_urn} → no transcodings");
        return Err(AppError::NotFound);
    }

    let futures = entries
        .into_iter()
        .map(|e| resolve_entry(&state, e))
        .collect::<Vec<_>>();
    let candidates: Vec<Candidate> = futures::future::join_all(futures)
        .await
        .into_iter()
        .flatten()
        .collect();

    if candidates.is_empty() {
        warn!("[download] {track_urn} → no usable candidates");
        return Err(AppError::NoStream);
    }

    Ok(Json(DownloadResponse {
        track_urn,
        candidates,
    }))
}

async fn collect_entries(state: &AppState, track_urn: &str, is_premium: bool) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut anon_count = 0usize;
    let mut cookies_count = 0usize;

    match state.anon.fetch_track_meta(track_urn).await {
        Ok((tcs, track_auth, cid)) => {
            for t in tcs {
                if seen.insert(t.url.clone()) {
                    anon_count += 1;
                    entries.push(Entry {
                        t,
                        client_id: cid.clone(),
                        track_auth: track_auth.clone(),
                        headers: HashMap::new(),
                    });
                }
            }
        }
        Err(e) => warn!("[download] {track_urn} anon meta failed: {e}"),
    }

    let mut cookies_status: &str = "skipped (not premium)";
    if is_premium {
        match state.cookies.as_ref() {
            None => cookies_status = "skipped (cookies disabled on server)",
            Some(cookies) => match cookies.fetch_track_meta(track_urn).await {
                Ok((tcs, track_auth, cid, h)) => {
                    cookies_status = "ok";
                    for t in tcs {
                        if seen.insert(t.url.clone()) {
                            cookies_count += 1;
                            entries.push(Entry {
                                t,
                                client_id: cid.clone(),
                                track_auth: track_auth.clone(),
                                headers: h.clone(),
                            });
                        }
                    }
                }
                Err(e) => {
                    warn!("[download] {track_urn} cookies meta failed: {e}");
                    cookies_status = "failed";
                }
            },
        }
    }

    info!(
        "[download] {track_urn} sources: anon={anon_count}, cookies={cookies_count} ({cookies_status})"
    );
    entries
}

fn free_tier(asked: Option<&str>) -> &'static str {
    granted_tier(asked, false).unwrap_or(quality::SQ)
}

async fn free_tier_entries(
    state: &AppState,
    track_urn: &str,
    session_id: &str,
    tier: &'static str,
) -> Result<(Vec<Entry>, Option<Arc<FreeSlot>>), AppError> {
    let slot = take_slot(&state.free_hq, track_urn, session_id, tier)?;
    let mut entries = collect_entries(state, track_urn, false).await;
    let answered = !entries.is_empty();
    entries.retain(|e| quality::of_transcoding(&e.t) == tier);
    if answered && entries.is_empty() {
        info!("[download] {track_urn} → no {tier}");
        return Err(AppError::NoStream);
    }
    Ok((entries, slot))
}

fn take_slot(
    free_hq: &FreeHq,
    track_urn: &str,
    session_id: &str,
    tier: &str,
) -> Result<Option<Arc<FreeSlot>>, AppError> {
    if tier == quality::SQ {
        return Ok(None);
    }
    let gate = free_hq.download_gate(track_urn, session_id);
    free_slot(gate, "[download]", track_urn, "download")
}

async fn resolve_entry(state: &AppState, entry: Entry) -> Option<Candidate> {
    let protocol = entry
        .t
        .format
        .as_ref()
        .and_then(|f| f.protocol.as_deref())
        .unwrap_or("");
    let mime = entry
        .t
        .format
        .as_ref()
        .and_then(|f| f.mime_type.as_deref())
        .unwrap_or("audio/mpeg")
        .to_string();
    let quality = quality::of_transcoding(&entry.t).to_string();
    let preset = entry
        .t
        .preset
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    if entry.t.snipped.unwrap_or(false) || entry.t.url.contains("/preview") {
        return None;
    }
    if entry.t.quality.as_deref() == Some("lq") {
        return None;
    }

    let target =
        build_transcoding_target(&entry.t.url, &entry.client_id, entry.track_auth.as_deref())?;

    let resp: ResolveResp = match fetch_get_json(
        &state.http_client,
        &state.config.sc_proxy_url,
        &target,
        entry.headers.clone(),
        false,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log_resolve_failure(&e.to_string(), "resolve", &preset, protocol);
            return None;
        }
    };

    match protocol {
        "progressive" => Some(Candidate::Progressive {
            quality,
            preset,
            mime,
            url: resp.url,
        }),
        "hls" => Some(Candidate::Hls {
            quality,
            preset,
            mime,
            manifest_url: resp.url,
        }),
        "ctr-encrypted-hls" => {
            let token = resp.auth_token?;
            prepare_encrypted(state, quality, preset, mime, resp.url, token, entry.headers).await
        }
        _ => None,
    }
}

async fn prepare_encrypted(
    state: &AppState,
    quality: String,
    preset: String,
    mime: String,
    manifest_url: String,
    token: String,
    headers: HashMap<String, String>,
) -> Option<Candidate> {
    let engine = state.decryptor.as_ref()?;

    let (manifest, _) = match fetch_get_text(
        &state.http_client,
        &state.config.sc_proxy_url,
        &manifest_url,
        headers.clone(),
        false,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log_resolve_failure(&e.to_string(), "manifest", &preset, "ctr-encrypted-hls");
            return None;
        }
    };

    let fetcher: Arc<dyn decrypt::Fetcher> = Arc::new(SegmentFetcher {
        client: state.http_client.clone(),
        proxy_url: state.config.sc_proxy_url.clone(),
        headers,
    });

    let prep = match engine.prepare_for_client(&manifest, &token, fetcher).await {
        Ok(p) => p,
        Err(e) => {
            if !e.is_disabled() {
                warn!("[download] prepare encrypted failed: {e}");
            }
            return None;
        }
    };

    let b64 = base64::engine::general_purpose::STANDARD;
    Some(Candidate::EncryptedHls {
        quality,
        preset,
        mime,
        content_type: prep.content_type,
        init_base64: b64.encode(&prep.init),
        segments: prep.segment_urls,
        key_base64: b64.encode(prep.key),
    })
}

struct SegmentFetcher {
    client: Client,
    proxy_url: String,
    headers: HashMap<String, String>,
}

impl decrypt::Fetcher for SegmentFetcher {
    fn get(
        &self,
        url: String,
        extra: Vec<(String, String)>,
    ) -> BoxFuture<'static, Result<Bytes, decrypt::Error>> {
        let client = self.client.clone();
        let proxy_url = self.proxy_url.clone();
        let mut merged = self.headers.clone();
        for (k, v) in extra {
            merged.insert(k, v);
        }
        Box::pin(async move {
            let (b, _) = fetch_get_bytes(&client, &proxy_url, &url, merged, false)
                .await
                .map_err(|e| decrypt::Error::Fetch(e.to_string()))?;
            Ok(b)
        })
    }

    fn post(
        &self,
        url: String,
        extra: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> BoxFuture<'static, Result<Bytes, decrypt::Error>> {
        let client = self.client.clone();
        let proxy_url = self.proxy_url.clone();
        let mut merged = self.headers.clone();
        for (k, v) in extra {
            merged.insert(k, v);
        }
        Box::pin(async move {
            let (b, _) = super::proxy::fetch_post_bytes(&client, &proxy_url, &url, merged, body)
                .await
                .map_err(|e| decrypt::Error::Fetch(e.to_string()))?;
            Ok(b)
        })
    }
}

fn classify_status(msg: &str) -> Option<u16> {
    for pat in ["relay status ", "status "] {
        if let Some(i) = msg.find(pat) {
            let tail = &msg[i + pat.len()..];
            let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u16>() {
                return Some(n);
            }
        }
    }
    None
}

fn log_resolve_failure(msg: &str, stage: &str, preset: &str, protocol: &str) {
    match classify_status(msg) {
        Some(404) | Some(410) => debug!("[download] {stage} {preset}/{protocol} gone: {msg}"),
        Some(s) => warn!("[download] {stage} {preset}/{protocol} status {s}: {msg}"),
        None => warn!("[download] {stage} {preset}/{protocol} failed: {msg}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URN: &str = "soundcloud:tracks:1";
    const SESSION: &str = "session";

    #[test]
    fn a_free_download_without_quality_is_sq_and_waits_for_no_slot() {
        let full = FreeHq::new(0);

        assert_eq!(
            free_tier(None),
            quality::SQ,
            "8.5.1 and every sq ask of the new desktop send no quality and never play hq from it"
        );
        assert!(
            take_slot(&full, URN, SESSION, free_tier(None)).is_ok_and(|slot| slot.is_none()),
            "old clients must not keep the hq slots busy for the ones that ask hq"
        );
        assert_eq!(free_tier(Some("sq")), quality::SQ);
        assert_eq!(free_tier(Some("uq")), quality::HQ);
    }

    #[test]
    fn a_free_hq_download_needs_a_slot_and_reads_the_miss_list() {
        let free = FreeHq::new(1);
        let held = take_slot(&free, URN, SESSION, quality::HQ).unwrap();
        assert!(held.is_some());
        assert!(matches!(
            take_slot(&free, URN, "another", quality::HQ),
            Err(AppError::FreeHqBusy(_))
        ));
        drop(held);

        free.note_miss(URN);
        assert!(matches!(
            take_slot(&free, URN, SESSION, quality::HQ),
            Err(AppError::NoStream)
        ));
    }

    #[test]
    fn a_free_ask_above_sq_is_turned_away_before_soundcloud_hears_of_it() {
        let body = include_str!("download.rs")
            .split_once("async fn free_tier_entries(")
            .expect("the free tier path is still here")
            .1;
        let body = body.split_once("\n}\n").expect("the function ends").0;
        let gate = body
            .find("take_slot(")
            .expect("a free ask above sq waits for a slot");
        let meta = body
            .find("collect_entries(")
            .expect("the free tier path still reads the track");
        assert!(
            gate < meta,
            "with every slot busy the answer is 429 and costs SoundCloud nothing; reading the \
             track first spends a meta request and an sq license on a reply nobody uses"
        );
        assert!(
            !body.contains("note_miss"),
            "anon meta alone does not know about apiv1 hq; only /stream, which tries every \
             source, may mark a track as having no hq"
        );
    }

    #[test]
    fn classify_status_parses_known_formats() {
        assert_eq!(classify_status("status 404"), Some(404));
        assert_eq!(classify_status("status 502"), Some(502));
        assert_eq!(classify_status("relay status 429"), Some(429));
        assert_eq!(classify_status("fetch: status 410"), Some(410));
        assert_eq!(classify_status("send: connection reset"), None);
        assert_eq!(classify_status("timeout"), None);
        assert_eq!(classify_status("parse: expected `,`"), None);
    }
}
