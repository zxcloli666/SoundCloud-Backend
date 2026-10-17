use std::collections::HashMap;
use wreq::Client;

use super::proxy::{fetch_get_json, fetch_get_text};
use super::quality;

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, serde::Deserialize)]
pub struct TranscodingFormat {
    pub protocol: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Transcoding {
    pub url: String,
    pub preset: Option<String>,
    pub snipped: Option<bool>,
    pub quality: Option<String>,
    pub format: Option<TranscodingFormat>,
}

pub(crate) struct RestrictedSource {
    pub manifest: String,
    pub token: String,
    pub content_type: &'static str,
    pub quality: &'static str,
}

#[derive(serde::Deserialize)]
struct ResolveResp {
    url: String,
    #[serde(rename = "licenseAuthToken")]
    auth_token: Option<String>,
}

pub fn build_transcoding_target(
    transcoding_url: &str,
    client_id: &str,
    track_authorization: Option<&str>,
) -> Option<String> {
    let transcoding_url = super::target::soundcloud_api(transcoding_url)?;
    let transcoding_url = transcoding_url.as_str();
    let sep = if transcoding_url.contains('?') {
        "&"
    } else {
        "?"
    };
    let mut target = format!("{transcoding_url}{sep}client_id={client_id}");
    if let Some(auth) = track_authorization.filter(|a| !a.is_empty()) {
        target.push_str("&track_authorization=");
        target.push_str(auth);
    }
    Some(target)
}

fn content_type_from_mime(mime: Option<&str>) -> &'static str {
    match mime.map(|m| m.split(';').next().unwrap_or("").trim()) {
        Some("audio/mpeg") => "audio/mpeg",
        Some("audio/ogg") => "audio/ogg",
        _ => "audio/mp4",
    }
}

fn is_full_encrypted(t: &Transcoding) -> bool {
    t.format.as_ref().and_then(|f| f.protocol.as_deref()) == Some("ctr-encrypted-hls")
        && !t.snipped.unwrap_or(false)
}

pub(crate) fn pick_encrypted<'a>(
    transcodings: &'a [Transcoding],
    uq_first: bool,
    at_least: &str,
) -> Option<&'a Transcoding> {
    let full: Vec<&Transcoding> = transcodings
        .iter()
        .filter(|t| is_full_encrypted(t))
        .collect();
    let best = |allow_uq: bool| {
        full.iter()
            .copied()
            .filter(|t| t.quality.as_deref() != Some("lq"))
            .filter(|t| allow_uq || quality::of_transcoding(t) != quality::UQ)
            .max_by_key(|t| quality::rank(quality::of_transcoding(t)))
    };
    best(uq_first)
        .or_else(|| best(true))
        .or_else(|| full.first().copied())
        .filter(|t| quality::rank(quality::of_transcoding(t)) >= quality::rank(at_least))
}

pub(crate) async fn resolve(
    client: &Client,
    proxy_url: &str,
    tc: &Transcoding,
    client_id: &str,
    track_auth: Option<&str>,
    headers: HashMap<String, String>,
) -> Result<Option<RestrictedSource>, BoxErr> {
    let content_type =
        content_type_from_mime(tc.format.as_ref().and_then(|f| f.mime_type.as_deref()));
    let quality = quality::of_transcoding(tc);

    let Some(target) = build_transcoding_target(&tc.url, client_id, track_auth) else {
        return Ok(None);
    };
    let r: ResolveResp = fetch_get_json(client, proxy_url, &target, headers.clone(), false).await?;
    let token = match r.auth_token {
        Some(t) => t,
        None => return Ok(None),
    };
    let (manifest, _) = fetch_get_text(client, proxy_url, &r.url, headers, false).await?;

    Ok(Some(RestrictedSource {
        manifest,
        token,
        content_type,
        quality,
    }))
}

#[cfg(test)]
mod tests {
    use super::{Transcoding, pick_encrypted};
    use crate::stream::quality;

    fn encrypted(quality: &str, snipped: bool) -> Transcoding {
        let preset = if quality == "hq" {
            "aac_256k"
        } else {
            "aac_160k"
        };
        encrypted_preset(preset, quality, snipped)
    }

    fn encrypted_preset(preset: &str, quality: &str, snipped: bool) -> Transcoding {
        serde_json::from_value(serde_json::json!({
            "url": format!("https://api-v2.soundcloud.com/media/soundcloud:tracks:1/{preset}/stream/ctr-encrypted-hls"),
            "preset": preset,
            "snipped": snipped,
            "quality": quality,
            "format": { "protocol": "ctr-encrypted-hls", "mime_type": "audio/mp4" }
        }))
        .unwrap()
    }

    #[test]
    fn a_snipped_preview_is_never_served_as_the_whole_track() {
        let previews = [encrypted("hq", true), encrypted("sq", true)];
        assert!(pick_encrypted(&previews, true, quality::SQ).is_none());

        let mixed = [encrypted("hq", true), encrypted("sq", false)];
        let picked = pick_encrypted(&mixed, true, quality::SQ).unwrap();
        assert_eq!(picked.quality.as_deref(), Some("sq"));
        assert_eq!(picked.snipped, Some(false));
    }

    #[test]
    fn the_best_full_transcoding_wins_when_quality_comes_first() {
        let tcs = [
            encrypted_preset("abr_sq", "sq", false),
            encrypted_preset("aac_96k", "lq", false),
            encrypted_preset("aac_160k", "sq", false),
        ];
        let picked = pick_encrypted(&tcs, true, quality::SQ).unwrap();
        assert_eq!(picked.preset.as_deref(), Some("aac_160k"));
        assert_eq!(quality::of_transcoding(picked), quality::HQ);
    }

    #[test]
    fn economy_skips_uq_but_still_takes_aac_160k() {
        let tcs = [
            encrypted_preset("aac_256k", "hq", false),
            encrypted_preset("aac_160k", "sq", false),
            encrypted_preset("abr_sq", "sq", false),
        ];
        assert_eq!(
            pick_encrypted(&tcs, false, quality::HQ)
                .unwrap()
                .preset
                .as_deref(),
            Some("aac_160k")
        );
        assert_eq!(
            pick_encrypted(&tcs, true, quality::HQ)
                .unwrap()
                .preset
                .as_deref(),
            Some("aac_256k")
        );
    }

    #[test]
    fn a_free_sq_decrypt_still_takes_aac_160k() {
        let tcs = [
            encrypted_preset("abr_sq", "sq", false),
            encrypted_preset("aac_96k", "lq", false),
            encrypted_preset("aac_160k", "sq", false),
        ];
        assert_eq!(
            pick_encrypted(&tcs, false, quality::SQ)
                .unwrap()
                .preset
                .as_deref(),
            Some("aac_160k"),
            "soundcloud answers an encrypted abr_sq with 404 and aac_96k costs the same license and \
             decrypt, so this hq copy is served outside FREE_HQ_FETCHES on purpose"
        );
    }

    #[test]
    fn a_low_quality_stream_is_only_the_last_resort() {
        let only_lq = [encrypted_preset("aac_96k", "lq", false)];
        assert_eq!(
            pick_encrypted(&only_lq, true, quality::SQ)
                .unwrap()
                .preset
                .as_deref(),
            Some("aac_96k")
        );
    }

    #[test]
    fn a_track_below_the_wanted_tier_is_dropped_before_any_request() {
        let sq_only = [
            encrypted_preset("abr_sq", "sq", false),
            encrypted_preset("aac_96k", "lq", false),
        ];
        assert!(pick_encrypted(&sq_only, false, quality::HQ).is_none());
        assert!(pick_encrypted(&sq_only, true, quality::HQ).is_none());
        assert_eq!(
            pick_encrypted(&sq_only, false, quality::SQ)
                .unwrap()
                .preset
                .as_deref(),
            Some("abr_sq")
        );
    }
}
