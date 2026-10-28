use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};
use wreq::Client;

use super::egress::{Egress, Known, steered};
use super::hls::{M3u8Refresher, download_hls, download_progressive, fetch_m3u8_source};
use super::proxy::{fetch_get_json, fetch_get_text};
use super::quality;
pub(crate) use super::restricted::{Transcoding, build_transcoding_target};

const SC_BASE_URL: &str = "https://soundcloud.com";
const SC_API_V2: &str = "https://api-v2.soundcloud.com";
const CLIENT_ID_MIN_REFRESH: Duration = Duration::from_secs(30);

#[derive(Debug, serde::Deserialize)]
pub struct TrackMedia {
    pub transcodings: Option<Vec<Transcoding>>,
}

#[derive(serde::Deserialize)]
pub struct ResolvedTrack {
    pub permalink_url: Option<String>,
    pub track_authorization: Option<String>,
    pub media: Option<TrackMedia>,
    pub policy: Option<String>,
}

impl ResolvedTrack {
    fn closed_here(&self) -> bool {
        matches!(self.policy.as_deref(), Some("BLOCK" | "SNIP"))
    }

    fn transcodings(&self) -> Option<&Vec<Transcoding>> {
        self.media
            .as_ref()
            .and_then(|m| m.transcodings.as_ref())
            .filter(|t| !t.is_empty())
    }
}

pub(crate) type TrackMeta = (
    Vec<Transcoding>,
    Option<String>,
    String,
    HashMap<String, String>,
);

pub(crate) struct TrackRead {
    pub track: ResolvedTrack,
    pub region: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct TranscodingResolveResponse {
    url: String,
}

pub struct AnonStreamResult {
    pub data: Bytes,
    pub content_type: &'static str,
    pub quality: &'static str,
}

pub struct AnonClient {
    client: Client,
    proxy_url: String,
    client_id: Arc<RwLock<Option<String>>>,
    refresh_gate: Mutex<Option<Instant>>,
    egress: Egress,
}

impl AnonClient {
    pub fn new(client: Client, proxy_url: String) -> Self {
        Self {
            client,
            proxy_url,
            client_id: Arc::new(RwLock::new(None)),
            refresh_gate: Mutex::new(None),
            egress: Egress::new(Vec::new()),
        }
    }

    pub fn with_regions(mut self, regions: Vec<String>) -> Self {
        self.egress = Egress::new(regions);
        self
    }

    pub async fn get_client_id(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        {
            let cached = self.client_id.read().await;
            if let Some(ref id) = *cached {
                return Ok(id.clone());
            }
        }
        self.refresh_client_id().await
    }

    pub async fn get_track_by_id(
        &self,
        track_id: &str,
    ) -> Result<ResolvedTrack, Box<dyn std::error::Error + Send + Sync>> {
        let client_id = self.get_client_id().await?;
        let target = format!("{SC_API_V2}/tracks/{track_id}?client_id={client_id}");

        match fetch_get_json::<ResolvedTrack>(
            &self.client,
            &self.proxy_url,
            &target,
            HashMap::new(),
            false,
        )
        .await
        {
            Ok(t) => Ok(t),
            Err(_) => {
                let new_id = self.invalidate_and_refresh().await?;
                let retry_target = format!("{SC_API_V2}/tracks/{track_id}?client_id={new_id}");
                fetch_get_json(
                    &self.client,
                    &self.proxy_url,
                    &retry_target,
                    HashMap::new(),
                    false,
                )
                .await
            }
        }
    }

    pub(crate) async fn read_track(
        &self,
        track_id: &str,
    ) -> Result<TrackRead, Box<dyn std::error::Error + Send + Sync>> {
        let known = self.egress.known(track_id);
        if let Known::Region(region) = &known {
            match self.get_track_from(track_id, region).await {
                Ok(track) if !track.closed_here() => {
                    return Ok(TrackRead {
                        track,
                        region: Some(region.clone()),
                    });
                }
                _ => self.egress.forget(track_id),
            }
        }

        let track = self.get_track_by_id(track_id).await?;
        if !track.closed_here() || known == Known::Nowhere {
            return Ok(TrackRead {
                track,
                region: None,
            });
        }

        let (open, every_region_answered) = self.first_open_region(track_id).await;
        if let Some(open) = open {
            info!(
                "[anon] {track_id} is blocked where we asked first, {} sees it",
                open.region.as_deref().unwrap_or("?")
            );
            return Ok(open);
        }
        if every_region_answered {
            self.egress.remember_nowhere(track_id);
        }
        Ok(TrackRead {
            track,
            region: None,
        })
    }

    async fn first_open_region(&self, track_id: &str) -> (Option<TrackRead>, bool) {
        let mut every_region_answered = true;
        for region in self.egress.regions() {
            match self.get_track_from(track_id, region).await {
                Ok(open) if !open.closed_here() => {
                    self.egress.remember(track_id, region);
                    let open = TrackRead {
                        track: open,
                        region: Some(region.clone()),
                    };
                    return (Some(open), every_region_answered);
                }
                Ok(_) => {}
                Err(e) if region_absent(&e.to_string()) => {
                    debug!("[anon] {track_id}: no live proxy in {region}");
                }
                Err(e) => {
                    debug!("[anon] {track_id} through {region} failed: {e}");
                    every_region_answered = false;
                }
            }
        }
        (None, every_region_answered)
    }

    async fn reread_after_a_regional_404(&self, track_id: &str) -> Option<TrackRead> {
        if self.egress.regions().is_empty() {
            return None;
        }
        let (open, _) = self.first_open_region(track_id).await;
        if let Some(open) = &open {
            info!(
                "[anon] {track_id} answered 404 where we resolved it, trying {}",
                open.region.as_deref().unwrap_or("?")
            );
        }
        open
    }

    async fn get_track_from(
        &self,
        track_id: &str,
        region: &str,
    ) -> Result<ResolvedTrack, Box<dyn std::error::Error + Send + Sync>> {
        let client_id = self.get_client_id().await?;
        let target = format!("{SC_API_V2}/tracks/{track_id}?client_id={client_id}");
        fetch_get_json(
            &self.client,
            &self.proxy_url,
            &target,
            steered(Some(region)),
            false,
        )
        .await
    }

    pub async fn resolve_url(
        &self,
        url: &str,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let client_id = self.get_client_id().await?;
        let target = build_resolve_target(url, &client_id);

        match fetch_get_json::<serde_json::Value>(
            &self.client,
            &self.proxy_url,
            &target,
            HashMap::new(),
            false,
        )
        .await
        {
            Ok(track) => Ok(track),
            Err(_) => {
                let new_id = self.invalidate_and_refresh().await?;
                let retry_target = build_resolve_target(url, &new_id);
                fetch_get_json(
                    &self.client,
                    &self.proxy_url,
                    &retry_target,
                    HashMap::new(),
                    false,
                )
                .await
            }
        }
    }

    pub async fn resolve_transcoding_url(
        &self,
        transcoding_url: &str,
        explicit_client_id: Option<&str>,
        track_authorization: Option<&str>,
        region: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if explicit_client_id.is_none() && region.is_none() {
            let cid = self.get_client_id().await.unwrap_or_default();
            if let Some(url) = crate::stream::proxy::transcoding_via_relay(
                transcoding_url,
                &cid,
                track_authorization,
            )
            .await
            {
                return Ok(url);
            }
        }

        let client_id = match explicit_client_id {
            Some(id) => id.to_string(),
            None => self.get_client_id().await?,
        };
        let Some(target) =
            build_transcoding_target(transcoding_url, &client_id, track_authorization)
        else {
            return Err("transcoding url does not belong to soundcloud".into());
        };

        match fetch_get_json::<TranscodingResolveResponse>(
            &self.client,
            &self.proxy_url,
            &target,
            steered(region),
            false,
        )
        .await
        {
            Ok(r) => Ok(r.url),
            Err(_) if explicit_client_id.is_none() => {
                let new_id = self.invalidate_and_refresh().await?;
                let Some(retry_target) =
                    build_transcoding_target(transcoding_url, &new_id, track_authorization)
                else {
                    return Err("transcoding url does not belong to soundcloud".into());
                };
                let r: TranscodingResolveResponse = fetch_get_json(
                    &self.client,
                    &self.proxy_url,
                    &retry_target,
                    steered(region),
                    false,
                )
                .await?;
                Ok(r.url)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn get_stream(
        self: &Arc<Self>,
        track_urn: &str,
        at_least: &'static str,
    ) -> Result<Option<AnonStreamResult>, Box<dyn std::error::Error + Send + Sync>> {
        let track_id = track_urn.rsplit(':').next().unwrap_or(track_urn);

        let read = self
            .read_track(track_id)
            .await
            .map_err(|e| format!("get track failed: {e}"))?;

        let read = match read.track.transcodings() {
            Some(_) => read,
            None if read.track.closed_here() => {
                return Err(format!("{track_id} is blocked in every region we read from").into());
            }
            None => {
                warn!("[anon] no transcodings for {track_id}, refreshing client_id");
                self.invalidate_and_refresh().await?;
                let retry = self
                    .read_track(track_id)
                    .await
                    .map_err(|e| format!("retry get track failed: {e}"))?;
                if retry.track.transcodings().is_none() {
                    return Err(format!(
                        "still no transcodings for {track_id} after a client_id refresh"
                    )
                    .into());
                }
                return self.stream_read(track_id, &retry, at_least).await;
            }
        };

        match self.stream_read(track_id, &read, at_least).await {
            Ok(found) => Ok(found),
            Err(e) => {
                warn!("[anon] stream failed for {track_id}, refreshing client_id: {e}");
                self.invalidate_and_refresh().await?;
                let retry = self
                    .read_track(track_id)
                    .await
                    .map_err(|e2| format!("retry get track failed: {e2}"))?;
                if retry.track.transcodings().is_none() {
                    return Err(format!(
                        "no transcodings for {track_id} after a client_id refresh"
                    )
                    .into());
                }
                self.stream_read(track_id, &retry, at_least).await
            }
        }
    }

    async fn stream_read(
        self: &Arc<Self>,
        track_id: &str,
        read: &TrackRead,
        at_least: &'static str,
    ) -> Result<Option<AnonStreamResult>, Box<dyn std::error::Error + Send + Sync>> {
        let found = self.stream_plain(read, at_least).await?;
        let plain_listed = read
            .track
            .transcodings()
            .is_some_and(|t| !ranked_transcodings(t, at_least).is_empty());
        if found.is_some() || read.region.is_some() || !plain_listed {
            return Ok(found);
        }
        match self.reread_after_a_regional_404(track_id).await {
            Some(open) => self.stream_plain(&open, at_least).await,
            None => Ok(None),
        }
    }

    async fn stream_plain(
        self: &Arc<Self>,
        read: &TrackRead,
        at_least: &'static str,
    ) -> Result<Option<AnonStreamResult>, Box<dyn std::error::Error + Send + Sync>> {
        let Some(transcodings) = read.track.transcodings() else {
            return Ok(None);
        };
        self.stream_from_transcodings(
            transcodings,
            read.track.track_authorization.as_deref(),
            at_least,
            read.region.as_deref(),
        )
        .await
    }

    async fn stream_from_transcodings(
        self: &Arc<Self>,
        transcodings: &[Transcoding],
        track_auth: Option<&str>,
        at_least: &'static str,
        region: Option<&str>,
    ) -> Result<Option<AnonStreamResult>, Box<dyn std::error::Error + Send + Sync>> {
        let ranked = ranked_transcodings(transcodings, at_least);
        if ranked.is_empty() {
            return Ok(None);
        }

        let mut last_err: Option<Box<dyn std::error::Error + Send + Sync>> = None;
        let mut only_resource_gone = true;
        for t in ranked {
            let mime = t
                .format
                .as_ref()
                .and_then(|f| f.mime_type.as_deref())
                .unwrap_or("audio/mpeg");
            let is_progressive =
                t.format.as_ref().and_then(|f| f.protocol.as_deref()) == Some("progressive");

            let media_url = match self
                .resolve_transcoding_url(&t.url, None, track_auth, region)
                .await
            {
                Ok(u) => u,
                Err(e) => {
                    warn!(
                        "[anon] resolve {} failed: {e}",
                        t.preset.as_deref().unwrap_or("?")
                    );
                    if !looks_like_resource_gone(&e.to_string()) {
                        only_resource_gone = false;
                    }
                    last_err = Some(e);
                    continue;
                }
            };

            let result = if is_progressive {
                download_progressive(
                    &self.client,
                    &self.proxy_url,
                    &media_url,
                    mime,
                    HashMap::new(),
                    false,
                )
                .await
            } else {
                let refresher: M3u8Refresher = {
                    let me = Arc::clone(self);
                    let transcoding_url = t.url.clone();
                    let track_auth = track_auth.map(str::to_string);
                    let region = region.map(str::to_string);
                    Arc::new(move || {
                        let me = Arc::clone(&me);
                        let transcoding_url = transcoding_url.clone();
                        let track_auth = track_auth.clone();
                        let region = region.clone();
                        Box::pin(async move {
                            let media = me
                                .resolve_transcoding_url(
                                    &transcoding_url,
                                    None,
                                    track_auth.as_deref(),
                                    region.as_deref(),
                                )
                                .await?;
                            fetch_m3u8_source(
                                &me.client,
                                &me.proxy_url,
                                &media,
                                HashMap::new(),
                                false,
                            )
                            .await
                        })
                    })
                };
                download_hls(
                    &self.client,
                    &self.proxy_url,
                    &media_url,
                    mime,
                    HashMap::new(),
                    false,
                    Some(refresher),
                )
                .await
            };

            match result {
                Ok((data, content_type)) => {
                    return Ok(Some(AnonStreamResult {
                        data,
                        content_type,
                        quality: quality::of_transcoding(t),
                    }));
                }
                Err(e) => {
                    warn!(
                        "[anon] transcoding {} ({}) failed: {e}",
                        t.preset.as_deref().unwrap_or("?"),
                        if is_progressive { "progressive" } else { "hls" },
                    );
                    if !looks_like_resource_gone(&e.to_string()) {
                        only_resource_gone = false;
                    }
                    last_err = Some(e);
                }
            }
        }

        if only_resource_gone {
            warn!("[anon] no usable transcoding for track");
            return Ok(None);
        }

        Err(last_err.unwrap_or_else(|| "all anon transcodings failed".into()))
    }

    pub(crate) async fn resolve_restricted(
        &self,
        track_urn: &str,
        uq_first: bool,
        at_least: &str,
    ) -> Result<Option<super::restricted::RestrictedSource>, Box<dyn std::error::Error + Send + Sync>>
    {
        let track_id = track_urn.rsplit(':').next().unwrap_or(track_urn);
        let read = self.read_track(track_id).await?;
        match self.resolve_encrypted(&read, uq_first, at_least).await {
            Err(e) if read.region.is_none() && looks_like_resource_gone(&e.to_string()) => {
                let Some(open) = self.reread_after_a_regional_404(track_id).await else {
                    return Err(e);
                };
                let resolved = self.resolve_encrypted(&open, uq_first, at_least).await;
                if !matches!(resolved, Ok(Some(_))) {
                    self.egress.forget(track_id);
                }
                resolved
            }
            resolved => resolved,
        }
    }

    async fn resolve_encrypted(
        &self,
        read: &TrackRead,
        uq_first: bool,
        at_least: &str,
    ) -> Result<Option<super::restricted::RestrictedSource>, Box<dyn std::error::Error + Send + Sync>>
    {
        let Some(tcs) = read.track.transcodings() else {
            return Ok(None);
        };
        let cid = self.get_client_id().await?;
        let mut refused = None;
        for tc in super::restricted::encrypted_in_order(tcs, uq_first, at_least) {
            match super::restricted::resolve(
                &self.client,
                &self.proxy_url,
                tc,
                &cid,
                read.track.track_authorization.as_deref(),
                steered(read.region.as_deref()),
            )
            .await
            {
                Err(e) if looks_like_resource_gone(&e.to_string()) => refused = Some(e),
                Ok(None) => {}
                found => return found,
            }
        }
        refused.map_or(Ok(None), Err)
    }

    pub(crate) async fn fetch_track_meta(
        &self,
        track_urn: &str,
    ) -> Result<TrackMeta, Box<dyn std::error::Error + Send + Sync>> {
        let track_id = track_urn.rsplit(':').next().unwrap_or(track_urn);
        let read = self.read_track(track_id).await?;
        let cid = self.get_client_id().await?;
        let headers = steered(read.region.as_deref());
        let tcs = read
            .track
            .media
            .and_then(|m| m.transcodings)
            .ok_or("anon: no transcodings")?;
        Ok((tcs, read.track.track_authorization, cid, headers))
    }

    async fn refresh_client_id(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.coalesced_refresh().await
    }

    async fn invalidate_and_refresh(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.coalesced_refresh().await
    }

    async fn coalesced_refresh(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut gate = self.refresh_gate.lock().await;

        if let Some(last) = *gate
            && last.elapsed() < CLIENT_ID_MIN_REFRESH
            && let Some(id) = self.client_id.read().await.clone()
        {
            return Ok(id);
        }

        let client_id = self.fetch_client_id().await?;
        *self.client_id.write().await = Some(client_id.clone());
        *gate = Some(Instant::now());
        info!("Refreshed SoundCloud public client_id");
        Ok(client_id)
    }

    async fn fetch_client_id(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let headers = HashMap::new();

        let (html, _) =
            fetch_get_text(&self.client, &self.proxy_url, SC_BASE_URL, headers, false).await?;

        extract_client_id_from_hydration(&html)
            .ok_or_else(|| "Failed to extract SoundCloud client_id from page".into())
    }
}

fn ranked_transcodings<'a>(
    transcodings: &'a [Transcoding],
    at_least: &str,
) -> Vec<&'a Transcoding> {
    let floor = quality::rank(at_least);
    let candidates: Vec<&Transcoding> = transcodings
        .iter()
        .filter(|t| {
            let encrypted = t
                .format
                .as_ref()
                .and_then(|f| f.protocol.as_deref())
                .unwrap_or("")
                .contains("encrypted");
            !encrypted && !t.snipped.unwrap_or(false) && !t.url.contains("/preview")
        })
        .filter(|t| quality::rank(quality::of_transcoding(t)) >= floor)
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    let is_progressive = |t: &&Transcoding| {
        t.format.as_ref().and_then(|f| f.protocol.as_deref()) == Some("progressive")
    };

    const PRESET_ORDER: &[&str] = &["mp3_1_0", "aac_160k", "opus_0_0", "abr_sq"];

    let mut ordered: Vec<&Transcoding> = Vec::with_capacity(candidates.len());

    for preset in PRESET_ORDER {
        if let Some(t) = candidates
            .iter()
            .find(|t| is_progressive(t) && t.preset.as_deref() == Some(preset))
        {
            ordered.push(t);
        }
    }
    for t in &candidates {
        if is_progressive(t) && !ordered.iter().any(|o| std::ptr::eq(*o, *t)) {
            ordered.push(t);
        }
    }

    for preset in PRESET_ORDER {
        if let Some(t) = candidates
            .iter()
            .find(|t| !is_progressive(t) && t.preset.as_deref() == Some(preset))
        {
            ordered.push(t);
        }
    }
    for t in &candidates {
        if !ordered.iter().any(|o| std::ptr::eq(*o, *t)) {
            ordered.push(t);
        }
    }
    if floor > quality::rank(quality::SQ) {
        ordered.sort_by_key(|t| std::cmp::Reverse(quality::rank(quality::of_transcoding(t))));
    }
    ordered
}

fn extract_client_id_from_hydration(html: &str) -> Option<String> {
    let pattern = r#""hydratable"\s*:\s*"apiClient"\s*,\s*"data"\s*:\s*\{\s*"id"\s*:\s*"([^"]+)""#;
    let re = regex::Regex::new(pattern).ok()?;
    let caps = re.captures(html)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

fn build_resolve_target(url: &str, client_id: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("url", url)
        .append_pair("client_id", client_id)
        .finish();
    format!("{SC_API_V2}/resolve?{query}")
}

fn looks_like_resource_gone(err: &str) -> bool {
    err.contains("404")
}

fn region_absent(err: &str) -> bool {
    err == "status 502"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MEDIA: &str = r#"{
        "transcodings": [
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/0e383483-3590-44ee-a4a3-c25df162579f/stream/cbc-encrypted-hls", "preset": "aac_160k", "duration": 148821, "snipped": false, "quality": "sq", "is_legacy_transcoding": false, "format": { "protocol": "cbc-encrypted-hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\"" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/0e383483-3590-44ee-a4a3-c25df162579f/stream/ctr-encrypted-hls", "preset": "aac_160k", "duration": 148821, "snipped": false, "quality": "sq", "is_legacy_transcoding": false, "format": { "protocol": "ctr-encrypted-hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\"" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/31935ff7-e765-4086-8771-431fe2306b83/stream/cbc-encrypted-hls", "preset": "aac_96k", "duration": 148821, "snipped": false, "quality": "lq", "is_legacy_transcoding": false, "format": { "protocol": "cbc-encrypted-hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\"" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/31935ff7-e765-4086-8771-431fe2306b83/stream/ctr-encrypted-hls", "preset": "aac_96k", "duration": 148821, "snipped": false, "quality": "lq", "is_legacy_transcoding": false, "format": { "protocol": "ctr-encrypted-hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\"" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/66fd47a6-5e9f-4eb5-ad45-e2a5e6caa04d/stream/cbc-encrypted-hls", "preset": "abr_sq", "duration": 148821, "snipped": false, "quality": "sq", "is_legacy_transcoding": false, "format": { "protocol": "cbc-encrypted-hls", "mime_type": "audio/mpegurl" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/66fd47a6-5e9f-4eb5-ad45-e2a5e6caa04d/stream/ctr-encrypted-hls", "preset": "abr_sq", "duration": 148821, "snipped": false, "quality": "sq", "is_legacy_transcoding": false, "format": { "protocol": "ctr-encrypted-hls", "mime_type": "audio/mpegurl" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/1dc4586b-cb41-46d1-89c9-5aed528f6620/stream/hls", "preset": "mp3_1_0", "duration": 148846, "snipped": false, "quality": "sq", "is_legacy_transcoding": true, "format": { "protocol": "hls", "mime_type": "audio/mpeg" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:2028682452/1dc4586b-cb41-46d1-89c9-5aed528f6620/stream/progressive", "preset": "mp3_1_0", "duration": 148846, "snipped": false, "quality": "sq", "is_legacy_transcoding": true, "format": { "protocol": "progressive", "mime_type": "audio/mpeg" } }
        ]
    }"#;

    fn protocol(t: &Transcoding) -> &str {
        t.format
            .as_ref()
            .and_then(|f| f.protocol.as_deref())
            .unwrap_or("")
    }

    #[test]
    fn anon_picks_mp3_progressive_first_when_others_encrypted() {
        let media: TrackMedia = serde_json::from_str(SAMPLE_MEDIA).unwrap();
        let transcodings = media.transcodings.expect("transcodings present");

        let ranked = ranked_transcodings(&transcodings, quality::SQ);

        assert_eq!(
            ranked.len(),
            2,
            "only non-encrypted mp3_1_0 transcodings should remain"
        );
        assert!(
            ranked.iter().all(|t| !protocol(t).contains("encrypted")),
            "no encrypted transcoding may survive the filter"
        );

        let first = ranked[0];
        assert_eq!(protocol(first), "progressive");
        assert_eq!(first.preset.as_deref(), Some("mp3_1_0"));
        assert_eq!(
            first.format.as_ref().and_then(|f| f.mime_type.as_deref()),
            Some("audio/mpeg")
        );

        let second = ranked[1];
        assert_eq!(protocol(second), "hls");
        assert_eq!(second.preset.as_deref(), Some("mp3_1_0"));
    }

    #[test]
    fn asking_for_hq_leaves_only_aac_160k_and_better() {
        let transcodings: Vec<Transcoding> = serde_json::from_value(serde_json::json!([
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:1/a/stream/progressive", "preset": "mp3_1_0", "quality": "sq", "format": { "protocol": "progressive", "mime_type": "audio/mpeg" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:1/b/stream/hls", "preset": "aac_160k", "quality": "sq", "format": { "protocol": "hls", "mime_type": "audio/mp4" } },
            { "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:1/c/stream/hls", "preset": "opus_0_0", "quality": "sq", "format": { "protocol": "hls", "mime_type": "audio/ogg" } }
        ]))
        .unwrap();

        let economy = ranked_transcodings(&transcodings, quality::SQ);
        assert_eq!(economy.len(), 3);
        assert_eq!(economy[0].preset.as_deref(), Some("mp3_1_0"));

        let hq = ranked_transcodings(&transcodings, quality::HQ);
        assert_eq!(hq.len(), 1);
        assert_eq!(hq[0].preset.as_deref(), Some("aac_160k"));

        assert!(ranked_transcodings(&transcodings, quality::UQ).is_empty());
    }

    #[test]
    fn transcoding_target_includes_track_authorization() {
        let media: TrackMedia = serde_json::from_str(SAMPLE_MEDIA).unwrap();
        let transcodings = media.transcodings.expect("transcodings present");
        let ranked = ranked_transcodings(&transcodings, quality::SQ);
        let progressive = ranked[0];

        let with_auth = build_transcoding_target(&progressive.url, "CID", Some("TRACK_AUTH_JWT"));
        assert_eq!(
            with_auth.as_deref(),
            Some(
                format!(
                    "{}?client_id=CID&track_authorization=TRACK_AUTH_JWT",
                    progressive.url
                )
                .as_str()
            ),
            "policy-gated track must send track_authorization"
        );

        let without_auth = build_transcoding_target(&progressive.url, "CID", None);
        assert_eq!(
            without_auth.as_deref(),
            Some(format!("{}?client_id=CID", progressive.url).as_str())
        );

        let empty_auth = build_transcoding_target(&progressive.url, "CID", Some(""));
        assert_eq!(empty_auth, without_auth);

        let pre_query = build_transcoding_target(
            "https://api-v2.soundcloud.com/media/x/stream/progressive?foo=1",
            "CID",
            Some("AUTH"),
        );
        assert_eq!(
            pre_query.as_deref(),
            Some(
                "https://api-v2.soundcloud.com/media/x/stream/progressive?foo=1&client_id=CID&track_authorization=AUTH"
            )
        );
    }

    #[test]
    fn a_transcoding_that_points_anywhere_but_soundcloud_is_never_resolved() {
        for hostile in [
            "http://127.0.0.1:8080/media/x/stream/progressive",
            "https://169.254.169.254/latest/meta-data/",
            "https://api-v2.soundcloud.com@evil.example/media/x",
            "https://evil.example/media/x/stream/progressive",
        ] {
            assert_eq!(
                build_transcoding_target(hostile, "CID", Some("AUTH")),
                None,
                "{hostile} arrived inside a soundcloud answer, and resolving it would send our \
                 client_id and the listener's track_authorization to whoever put it there"
            );
        }
    }

    #[test]
    fn resource_gone_classifier() {
        assert!(looks_like_resource_gone("status 404"));
        assert!(looks_like_resource_gone("HTTP 404 Not Found"));
        assert!(!looks_like_resource_gone("status 401"));
        assert!(!looks_like_resource_gone("status 403"));
        assert!(!looks_like_resource_gone("status 502"));
        assert!(!looks_like_resource_gone("connection reset"));
    }

    #[tokio::test]
    #[ignore = "hits live SoundCloud; run with --ignored"]
    async fn restricted_track_has_no_plain_transcoding() {
        let http = Client::new();
        let ua = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36";

        let html = http
            .get(SC_BASE_URL)
            .header("User-Agent", ua)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let client_id =
            extract_client_id_from_hydration(&html).expect("scrape client_id from homepage");

        let track: ResolvedTrack = http
            .get(format!(
                "{SC_API_V2}/tracks/2028682452?client_id={client_id}"
            ))
            .header("User-Agent", ua)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        let track_auth = track
            .track_authorization
            .clone()
            .expect("api-v2 /tracks returns track_authorization");
        let transcodings = track.media.unwrap().transcodings.unwrap();

        let ranked = ranked_transcodings(&transcodings, quality::SQ);
        assert_eq!(ranked.len(), 2, "only the two mp3_1_0 entries survive");
        for t in &ranked {
            let target = build_transcoding_target(&t.url, &client_id, Some(&track_auth))
                .expect("soundcloud names its own media host");
            let status = http
                .get(&target)
                .header("User-Agent", ua)
                .send()
                .await
                .unwrap()
                .status();
            assert!(
                status.is_client_error(),
                "expected plain mp3 transcoding to be gone, got {status}"
            );
        }

        let enc = transcodings
            .iter()
            .find(|t| {
                t.format
                    .as_ref()
                    .and_then(|f| f.protocol.as_deref())
                    .is_some_and(|p| p.contains("encrypted"))
            })
            .expect("restricted transcoding present");
        let enc_status = http
            .get(
                build_transcoding_target(&enc.url, &client_id, Some(&track_auth))
                    .expect("soundcloud names its own media host"),
            )
            .header("User-Agent", ua)
            .send()
            .await
            .unwrap()
            .status();
        assert!(
            enc_status.is_success(),
            "restricted transcoding should resolve, got {enc_status}"
        );
    }
}
