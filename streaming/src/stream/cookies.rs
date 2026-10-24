use bytes::Bytes;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::{debug, info, warn};
use wreq::Client;

use std::sync::Arc;

use super::anon::AnonClient;
use super::hls::{M3u8Refresher, download_hls, download_progressive, fetch_m3u8_source};
use super::proxy::{fetch_get_json, fetch_get_text};
use super::quality;
use super::restricted::Transcoding;

const FAILURE_THRESHOLD: u32 = 3;

pub(crate) const PREVIEWS_ONLY: &str = "cookies: previews only";

pub struct CookieStreamResult {
    pub data: Bytes,
    pub content_type: &'static str,
    pub quality: &'static str,
}

pub struct CookiesClient {
    client: Client,
    proxy_url: String,
    cookies: String,
    oauth_token: String,
    anon: AnonClient,
    consecutive_failures: AtomicU32,
}

#[derive(serde::Deserialize)]
struct CookieHydrationSound {
    media: Option<CookieHydrationMedia>,
    track_authorization: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CookieHydrationMedia {
    transcodings: Option<Vec<Transcoding>>,
}

#[derive(serde::Deserialize)]
struct ResolveResp {
    url: String,
}

#[derive(serde::Deserialize)]
struct AuthedTrack {
    permalink_url: Option<String>,
}

impl CookiesClient {
    pub fn new(
        client: Client,
        proxy_url: String,
        cookies: String,
        oauth_token: String,
        anon: AnonClient,
    ) -> Self {
        Self {
            client,
            proxy_url,
            cookies,
            oauth_token,
            anon,
            consecutive_failures: AtomicU32::new(0),
        }
    }

    pub async fn get_stream(
        &self,
        track_urn: &str,
        hq_only: bool,
    ) -> Result<Option<CookieStreamResult>, Box<dyn std::error::Error + Send + Sync>> {
        let track_id = track_urn.rsplit(':').next().unwrap_or(track_urn);

        let track = self
            .anon
            .get_track_by_id(track_id)
            .await
            .map_err(|err| format!("anon track lookup failed ({err})"))?;
        let permalink = match track.permalink_url {
            Some(ref p) => p.clone(),
            None => {
                debug!("[cookies] no permalink for {track_id}");
                return Ok(None);
            }
        };

        let (sound, client_id) = match self.fetch_hydration(&permalink).await {
            Some((s, c)) => (s, c),
            None => return Ok(None),
        };

        let transcodings = match sound.media.and_then(|m| m.transcodings) {
            Some(t) if !t.is_empty() => t,
            _ => {
                debug!("[cookies] no transcodings for {track_id}");
                self.record_failure();
                return Ok(None);
            }
        };

        let track_auth = sound.track_authorization.unwrap_or_default();

        let full: Vec<&Transcoding> = transcodings.iter().filter(|t| is_full(t)).collect();

        if full.is_empty() {
            debug!("[cookies] no full transcodings for {track_id}");
            return Err(PREVIEWS_ONLY.into());
        }

        let ordered = plain_in_order(&full, hq_only);
        if ordered.is_empty() {
            debug!("[cookies] nothing plain to download for {track_id}");
            return Ok(None);
        }

        for transcoding in ordered {
            let quality = quality::of_transcoding(transcoding);

            match self
                .try_transcoding(transcoding, &track_auth, &client_id)
                .await
            {
                Ok((data, content_type)) => {
                    self.record_success();
                    return Ok(Some(CookieStreamResult {
                        data,
                        content_type,
                        quality,
                    }));
                }
                Err(e) => {
                    debug!(
                        "[cookies] transcoding {} failed: {e}",
                        transcoding.preset.as_deref().unwrap_or("?")
                    );
                }
            }
        }

        self.record_failure();
        Ok(None)
    }

    fn auth_headers(&self) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("Accept".into(), "*/*".into());
        h.insert(
            "Authorization".into(),
            format!("OAuth {}", self.oauth_token),
        );
        h.insert("Origin".into(), "https://soundcloud.com".into());
        h.insert("Referer".into(), "https://soundcloud.com/".into());
        h.insert(
            "User-Agent".into(),
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36".into(),
        );
        h
    }

    pub(crate) fn cookie_auth_headers(&self) -> HashMap<String, String> {
        self.auth_headers()
    }

    pub(crate) async fn fetch_track_meta(
        &self,
        track_urn: &str,
    ) -> Result<(Vec<Transcoding>, Option<String>, String), Box<dyn std::error::Error + Send + Sync>>
    {
        let track_id = track_urn.rsplit(':').next().unwrap_or(track_urn);
        let track: AuthedTrack = fetch_get_json(
            &self.client,
            &self.proxy_url,
            &format!("https://api-v2.soundcloud.com/tracks/{track_id}"),
            self.auth_headers(),
            false,
        )
        .await?;
        let permalink = track.permalink_url.ok_or("cookies: no permalink")?;
        let (sound, client_id) = self
            .fetch_hydration(&permalink)
            .await
            .ok_or("cookies: hydration failed")?;
        let tcs = sound
            .media
            .and_then(|m| m.transcodings)
            .ok_or("cookies: no transcodings")?;
        Ok((tcs, sound.track_authorization, client_id))
    }

    pub(crate) async fn resolve_restricted(
        &self,
        track_urn: &str,
        uq_first: bool,
        at_least: &str,
    ) -> Result<Option<super::restricted::RestrictedSource>, Box<dyn std::error::Error + Send + Sync>>
    {
        let track_id = track_urn.rsplit(':').next().unwrap_or(track_urn);
        let track: AuthedTrack = fetch_get_json(
            &self.client,
            &self.proxy_url,
            &format!("https://api-v2.soundcloud.com/tracks/{track_id}"),
            self.auth_headers(),
            false,
        )
        .await?;
        let permalink = match track.permalink_url {
            Some(p) => p,
            None => return Ok(None),
        };
        let (sound, client_id) = match self.fetch_hydration(&permalink).await {
            Some(v) => v,
            None => return Ok(None),
        };
        let tcs = match sound.media.and_then(|m| m.transcodings) {
            Some(t) if !t.is_empty() => t,
            _ => return Ok(None),
        };
        if !tcs.iter().any(is_full) {
            return Err(PREVIEWS_ONLY.into());
        }
        let Some(tc) = super::restricted::pick_encrypted(&tcs, uq_first, at_least) else {
            return Ok(None);
        };
        super::restricted::resolve(
            &self.client,
            &self.proxy_url,
            tc,
            &client_id,
            sound.track_authorization.as_deref(),
            self.auth_headers(),
        )
        .await
    }

    async fn try_transcoding(
        &self,
        transcoding: &Transcoding,
        track_auth: &str,
        client_id: &str,
    ) -> Result<(Bytes, &'static str), Box<dyn std::error::Error + Send + Sync>> {
        let transcoding_url = &transcoding.url;
        let sep = if transcoding_url.contains('?') {
            "&"
        } else {
            "?"
        };
        let target =
            format!("{transcoding_url}{sep}client_id={client_id}&track_authorization={track_auth}");

        let headers = self.auth_headers();

        let resp: ResolveResp = fetch_get_json(
            &self.client,
            &self.proxy_url,
            &target,
            headers.clone(),
            false,
        )
        .await?;

        let mime = transcoding
            .format
            .as_ref()
            .and_then(|f| f.mime_type.as_deref())
            .unwrap_or("audio/mpeg");
        let is_progressive = transcoding
            .format
            .as_ref()
            .and_then(|f| f.protocol.as_deref())
            == Some("progressive");

        if is_progressive {
            download_progressive(
                &self.client,
                &self.proxy_url,
                &resp.url,
                mime,
                HashMap::new(),
                false,
            )
            .await
        } else {
            let refresher: M3u8Refresher = {
                let client = self.client.clone();
                let proxy_url = self.proxy_url.clone();
                let target = target.clone();
                let headers = headers.clone();
                Arc::new(move || {
                    let client = client.clone();
                    let proxy_url = proxy_url.clone();
                    let target = target.clone();
                    let headers = headers.clone();
                    Box::pin(async move {
                        let resp: ResolveResp =
                            fetch_get_json(&client, &proxy_url, &target, headers, false).await?;
                        fetch_m3u8_source(&client, &proxy_url, &resp.url, HashMap::new(), false)
                            .await
                    })
                })
            };
            download_hls(
                &self.client,
                &self.proxy_url,
                &resp.url,
                mime,
                HashMap::new(),
                false,
                Some(refresher),
            )
            .await
        }
    }

    async fn fetch_hydration(&self, permalink_url: &str) -> Option<(CookieHydrationSound, String)> {
        let page = super::target::soundcloud_page(permalink_url)?;

        let mut headers = HashMap::new();
        headers.insert(
            "User-Agent".into(),
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36".into(),
        );
        headers.insert("Cookie".into(), self.cookies.clone());

        let (html, _) =
            fetch_get_text(&self.client, &self.proxy_url, page.as_str(), headers, false)
                .await
                .ok()?;

        extract_cookie_hydration_data(&html)
    }

    fn record_failure(&self) {
        let prev = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        let n = prev + 1;
        if n == FAILURE_THRESHOLD || (n > FAILURE_THRESHOLD && n.is_multiple_of(25)) {
            warn!("[cookies] consecutive failures: {n}");
        } else {
            debug!("[cookies] consecutive failures: {n}");
        }
    }

    fn record_success(&self) {
        let prev = self.consecutive_failures.swap(0, Ordering::Relaxed);
        if prev >= FAILURE_THRESHOLD {
            info!("[cookies] recovered after {prev} failures");
        }
    }
}

fn plain_in_order<'a>(full: &[&'a Transcoding], hq_only: bool) -> Vec<&'a Transcoding> {
    let plain: Vec<&Transcoding> = full.iter().copied().filter(|t| !is_encrypted(t)).collect();
    let is_progressive = |t: &&Transcoding| {
        t.format.as_ref().and_then(|f| f.protocol.as_deref()) == Some("progressive")
    };
    let is_hq = |t: &&Transcoding| t.quality.as_deref() == Some("hq");

    let mut ordered: Vec<&Transcoding> = Vec::with_capacity(plain.len());
    ordered.extend(plain.iter().filter(|t| is_hq(t) && is_progressive(t)));
    ordered.extend(plain.iter().filter(|t| is_hq(t) && !is_progressive(t)));
    if !hq_only {
        ordered.extend(plain.iter().filter(|t| !is_hq(t) && is_progressive(t)));
        ordered.extend(plain.iter().filter(|t| !is_hq(t) && !is_progressive(t)));
    }
    ordered
}

fn is_encrypted(t: &Transcoding) -> bool {
    t.format
        .as_ref()
        .and_then(|f| f.protocol.as_deref())
        .is_some_and(|p| p.contains("encrypted"))
}

fn is_full(t: &Transcoding) -> bool {
    !t.snipped.unwrap_or(false) && !t.url.contains("/preview")
}

fn extract_balanced_json(s: &str) -> Option<&str> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;

    for (i, ch) in s.char_indices() {
        if !in_str {
            match ch {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&s[..i + 1]);
                    }
                }
                _ => {}
            }
        } else {
            if ch == '"' && !esc {
                in_str = false;
            }
            esc = !esc && ch == '\\';
        }
    }
    None
}

fn extract_cookie_hydration_data(html: &str) -> Option<(CookieHydrationSound, String)> {
    let client_id_pattern =
        r#""hydratable"\s*:\s*"apiClient"\s*,\s*"data"\s*:\s*\{\s*"id"\s*:\s*"([^"]+)""#;
    let client_id_re = regex::Regex::new(client_id_pattern).ok()?;
    let client_id = client_id_re.captures(html)?.get(1)?.as_str().to_string();

    let sound_pattern = r#""hydratable"\s*:\s*"sound"\s*,\s*"data"\s*:\s*\{"#;
    let sound_re = regex::Regex::new(sound_pattern).ok()?;
    let sound_match = sound_re.find(html)?;
    let sound_start = sound_match.end() - 1;
    let rest = &html[sound_start..];

    let sound_json = extract_balanced_json(rest)?;
    let sound: CookieHydrationSound = match serde_json::from_str(sound_json) {
        Ok(s) => s,
        Err(e) => {
            warn!("[cookies] sound JSON parse failed: {e}");
            return None;
        }
    };

    Some((sound, client_id))
}

#[cfg(test)]
mod tests {
    use super::{Transcoding, plain_in_order};

    fn transcoding(preset: &str, quality: &str, protocol: &str) -> Transcoding {
        serde_json::from_value(serde_json::json!({
            "url": format!("https://api-v2.soundcloud.com/media/soundcloud:tracks:1/{preset}/stream/{protocol}"),
            "preset": preset,
            "quality": quality,
            "format": { "protocol": protocol, "mime_type": "audio/mp4" }
        }))
        .unwrap()
    }

    fn protocols(ordered: &[&Transcoding]) -> Vec<String> {
        ordered
            .iter()
            .map(|t| t.format.as_ref().unwrap().protocol.clone().unwrap())
            .collect()
    }

    #[test]
    fn encrypted_transcodings_are_never_downloaded_as_plain_audio() {
        let tcs = [
            transcoding("aac_256k", "hq", "ctr-encrypted-hls"),
            transcoding("aac_160k", "sq", "cbc-encrypted-hls"),
            transcoding("aac_160k", "sq", "ctr-encrypted-hls"),
        ];
        let full: Vec<&Transcoding> = tcs.iter().collect();
        assert!(plain_in_order(&full, true).is_empty());
        assert!(plain_in_order(&full, false).is_empty());
    }

    #[test]
    fn hq_comes_first_and_sq_only_when_asked() {
        let tcs = [
            transcoding("mp3_1_0", "sq", "progressive"),
            transcoding("aac_256k", "hq", "ctr-encrypted-hls"),
            transcoding("aac_256k", "hq", "hls"),
            transcoding("aac_160k", "sq", "hls"),
        ];
        let full: Vec<&Transcoding> = tcs.iter().collect();
        assert_eq!(protocols(&plain_in_order(&full, true)), ["hls"]);
        assert_eq!(
            protocols(&plain_in_order(&full, false)),
            ["hls", "progressive", "hls"]
        );
    }
}
