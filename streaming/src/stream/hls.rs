use bytes::{Bytes, BytesMut};
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{debug, warn};
use url::Url;
use wreq::Client;

use super::proxy::{BodyValidator, fetch_direct_validated, fetch_get_validated};
use super::validate::{is_valid_audio, is_valid_m3u8};

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

const HLS_CONCURRENCY: usize = 3;
const MAX_M3U8_REFRESH: usize = 2;

pub type SegmentSource = (Option<String>, Vec<String>);

pub type M3u8Refresher = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<SegmentSource, BoxErr>> + Send>> + Send + Sync,
>;

fn audio_validator() -> BodyValidator {
    Arc::new(|b: &[u8], _: &HashMap<String, String>| is_valid_audio(b))
}

fn m3u8_validator() -> BodyValidator {
    Arc::new(|b: &[u8], _: &HashMap<String, String>| is_valid_m3u8(b))
}

async fn fetch_validated(
    client: &Client,
    proxy_url: &str,
    target_url: &str,
    headers: HashMap<String, String>,
    direct_only: bool,
    validate: BodyValidator,
) -> Result<Bytes, BoxErr> {
    let (data, _) = if direct_only {
        fetch_direct_validated(client, target_url, headers, validate).await?
    } else {
        fetch_get_validated(client, proxy_url, target_url, headers, false, validate).await?
    };
    Ok(data)
}

pub fn parse_m3u8(content: &str, base_url: &str) -> SegmentSource {
    let Some(base) = super::target::public_media(base_url) else {
        return (None, Vec::new());
    };
    let mut init_url = None;
    let mut segment_urls = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if let Some(start) = line.find("#EXT-X-MAP:URI=\"") {
            let rest = &line[start + 16..];
            if let Some(end) = rest.find('"') {
                init_url = resolve_url(&rest[..end], &base);
            }
            continue;
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some(segment) = resolve_url(line, &base) else {
            return (None, Vec::new());
        };
        segment_urls.push(segment);
    }

    (init_url, segment_urls)
}

fn resolve_url(url: &str, base: &Url) -> Option<String> {
    let absolute = match base.join(url) {
        Ok(absolute) => absolute,
        Err(_) => return None,
    };
    super::target::public_media(absolute.as_str()).map(|url| url.to_string())
}

pub fn mime_to_content_type(mime: &str) -> &'static str {
    match mime {
        "audio/mpeg" | "audio/mpegurl" => "audio/mpeg",
        m if m.contains("mp4a") => "audio/mp4",
        m if m.contains("opus") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

pub async fn fetch_m3u8_source(
    client: &Client,
    proxy_url: &str,
    m3u8_url: &str,
    m3u8_headers: HashMap<String, String>,
    direct_only: bool,
) -> Result<SegmentSource, BoxErr> {
    let data = fetch_validated(
        client,
        proxy_url,
        m3u8_url,
        m3u8_headers,
        direct_only,
        m3u8_validator(),
    )
    .await?;
    let text = String::from_utf8_lossy(&data);
    let source = parse_m3u8(&text, m3u8_url);
    if source.1.is_empty() {
        return Err("no segments found in m3u8".into());
    }
    Ok(source)
}

pub async fn download_progressive(
    client: &Client,
    proxy_url: &str,
    url: &str,
    mime_type: &str,
    extra_headers: HashMap<String, String>,
    direct_only: bool,
) -> Result<(Bytes, &'static str), BoxErr> {
    if !direct_only
        && let Some(audio) = crate::stream::proxy::progressive_download_via_relay(url).await
        && audio
            .first()
            .is_some_and(|b| !matches!(b, b'{' | b'[' | b'<' | b' '))
    {
        return Ok((Bytes::from(audio), mime_to_content_type(mime_type)));
    }

    let data = fetch_validated(
        client,
        proxy_url,
        url,
        extra_headers,
        direct_only,
        audio_validator(),
    )
    .await?;
    Ok((data, mime_to_content_type(mime_type)))
}

pub async fn download_hls(
    client: &Client,
    proxy_url: &str,
    m3u8_url: &str,
    mime_type: &str,
    m3u8_headers: HashMap<String, String>,
    direct_only: bool,
    refresher: Option<M3u8Refresher>,
) -> Result<(Bytes, &'static str), BoxErr> {
    if !direct_only
        && let Some(audio) = crate::stream::proxy::hls_download_via_relay(m3u8_url).await
        && audio
            .first()
            .is_some_and(|b| !matches!(b, b'{' | b'[' | b'<' | b' '))
    {
        return Ok((Bytes::from(audio), mime_to_content_type(mime_type)));
    }

    let (init_url, mut segment_urls) =
        fetch_m3u8_source(client, proxy_url, m3u8_url, m3u8_headers, direct_only).await?;

    let mut buf = BytesMut::new();

    if let Some(ref init) = init_url {
        let data = fetch_validated(
            client,
            proxy_url,
            init,
            HashMap::new(),
            direct_only,
            audio_validator(),
        )
        .await?;
        if data.windows(4).any(|w| w == b"enca") {
            return Err("unsupported stream".into());
        }
        buf.extend_from_slice(&data);
    }

    let mut results: Vec<Option<Bytes>> = vec![None; segment_urls.len()];
    let mut refreshes_used = 0usize;

    loop {
        let pending: Vec<usize> = results
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_none())
            .map(|(i, _)| i)
            .collect();
        if pending.is_empty() {
            break;
        }

        let failed = fetch_segment_batch(
            client,
            proxy_url,
            &segment_urls,
            &pending,
            direct_only,
            &mut results,
        )
        .await;

        if failed.is_empty() {
            continue;
        }

        let Some(ref refresher) = refresher else {
            return Err(format!("hls: {} segment(s) unrecoverable", failed.len()).into());
        };
        if refreshes_used >= MAX_M3U8_REFRESH {
            return Err("hls: segments still failing after m3u8 refresh".into());
        }

        let (_, fresh_segments) = refresher().await?;
        if fresh_segments.len() != segment_urls.len() {
            return Err("hls: refreshed playlist has a different segment count".into());
        }
        refreshes_used += 1;
        warn!(
            "[hls] re-resolved playlist after {} failed segment(s) (refresh {}/{})",
            failed.len(),
            refreshes_used,
            MAX_M3U8_REFRESH
        );
        segment_urls = fresh_segments;
    }

    for chunk in results.into_iter().flatten() {
        buf.extend_from_slice(&chunk);
    }

    Ok((buf.freeze(), mime_to_content_type(mime_type)))
}

async fn fetch_segment_batch(
    client: &Client,
    proxy_url: &str,
    segment_urls: &[String],
    indices: &[usize],
    direct_only: bool,
    results: &mut [Option<Bytes>],
) -> Vec<usize> {
    let mut stream = futures::stream::iter(indices.iter().copied().map(|idx| {
        let client = client.clone();
        let proxy_url = proxy_url.to_string();
        let url = segment_urls[idx].clone();
        async move {
            let res = fetch_validated(
                &client,
                &proxy_url,
                &url,
                HashMap::new(),
                direct_only,
                audio_validator(),
            )
            .await;
            (idx, res)
        }
    }))
    .buffer_unordered(HLS_CONCURRENCY);

    let mut failed = Vec::new();
    while let Some((idx, res)) = stream.next().await {
        match res {
            Ok(data) => results[idx] = Some(data),
            Err(e) => {
                debug!("[hls] segment {idx} failed: {e}");
                failed.push(idx);
            }
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://cf-hls-media.sndcdn.com/media/0/1/playlist.m3u8";

    #[test]
    fn a_playlist_soundcloud_served_yields_the_segments_it_names() {
        let playlist = concat!(
            "#EXTM3U\n",
            "#EXT-X-MAP:URI=\"init.mp4\"\n",
            "#EXTINF:6.0,\n",
            "segment-0.m4s\n",
            "#EXTINF:6.0,\n",
            "https://cf-hls-media.sndcdn.com/media/0/1/segment-1.m4s\n",
        );

        let (init, segments) = parse_m3u8(playlist, BASE);

        assert_eq!(
            init.as_deref(),
            Some("https://cf-hls-media.sndcdn.com/media/0/1/init.mp4")
        );
        assert_eq!(
            segments,
            vec![
                "https://cf-hls-media.sndcdn.com/media/0/1/segment-0.m4s".to_owned(),
                "https://cf-hls-media.sndcdn.com/media/0/1/segment-1.m4s".to_owned(),
            ]
        );
    }

    #[test]
    fn a_playlist_that_names_a_segment_inside_our_network_yields_nothing() {
        for hostile in [
            "http://127.0.0.1:8080/secret",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.5/admin",
            "file:///etc/passwd",
        ] {
            let playlist = format!("#EXTM3U\n#EXTINF:6.0,\n{hostile}\n");

            assert_eq!(
                parse_m3u8(&playlist, BASE),
                (None, Vec::new()),
                "a playlist body is attacker-shaped content; {hostile} inside it must not \
                 become a request made from inside our network"
            );
        }
    }

    #[test]
    fn a_playlist_whose_own_address_we_do_not_trust_is_not_parsed_at_all() {
        let playlist = "#EXTM3U\n#EXTINF:6.0,\nsegment-0.m4s\n";

        assert_eq!(
            parse_m3u8(playlist, "http://127.0.0.1:8080/playlist.m3u8"),
            (None, Vec::new()),
            "relative segments resolve against the base, so an untrusted base made every \
             segment untrusted too, and a bad base used to fall back to https://localhost"
        );
    }
}
