use sc_mobile::UserPlan;
use serde::Deserialize;
use url::Url;

use super::{MISSED, PREVIEW, REFUSED};
use crate::stream::quality;

const FULL_SHARE: f64 = 0.9;
const CLEAR: &str = "hls";
const ENCRYPTED: &str = "ctr-encrypted-hls";
const ADAPTIVE: &str = "abr_";
const LOW: &str = "lq";
const SNIP: &str = "SNIP";
const BLOCK: &str = "BLOCK";
const PREVIEW_SEGMENTS: [&str; 2] = ["preview", "snippet"];

#[derive(Deserialize)]
struct Reply {
    data: Option<Data>,
}

#[derive(Deserialize)]
struct Data {
    tracks: Option<Vec<Track>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Track {
    urn: String,
    full_duration: Option<i64>,
    authorization: Option<Authorization>,
    transcodings: Option<Vec<Transcoding>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Authorization {
    snipped: Option<bool>,
    blocked: Option<bool>,
    sub_mid_tier: Option<bool>,
    sub_high_tier: Option<bool>,
    policy: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Transcoding {
    url: String,
    preset: Option<String>,
    duration_ms: Option<i64>,
    format: Option<Format>,
    quality: Option<String>,
    snipped: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Format {
    protocol: Option<String>,
    mime_type: Option<String>,
}

pub(super) struct Picked<'a> {
    pub url: &'a str,
    pub mime: &'a str,
    pub tier: &'static str,
    pub encrypted: bool,
}

pub(super) fn parse(body: &[u8], track_urn: &str) -> Option<Track> {
    let reply: Reply = serde_json::from_slice(body).ok()?;
    reply
        .data?
        .tracks?
        .into_iter()
        .find(|track| track.urn == track_urn)
}

pub(super) fn names_a_preview(url: &Url) -> bool {
    url.path_segments()
        .is_some_and(|mut segments| segments.any(|segment| PREVIEW_SEGMENTS.contains(&segment)))
}

impl Track {
    pub(super) fn playable_with(&self, plan: Option<&str>) -> Result<(), &'static str> {
        let Some(authorization) = &self.authorization else {
            return Ok(());
        };
        if authorization.blocked == Some(true) || authorization.policy.as_deref() == Some(BLOCK) {
            return Err(REFUSED);
        }
        if authorization.snipped == Some(true) {
            return Err(PREVIEW);
        }
        if authorization.policy.as_deref() == Some(SNIP) && !authorization.opened_by(plan) {
            return Err(PREVIEW);
        }
        Ok(())
    }

    pub(super) fn pick(
        &self,
        tier: &'static str,
        decrypts: bool,
    ) -> Result<Picked<'_>, &'static str> {
        let full_ms = self.full_ms()?;
        let at_tier: Vec<&Transcoding> = self
            .transcodings
            .iter()
            .flatten()
            .filter(|transcoding| transcoding.tier() == tier && transcoding.streams(decrypts))
            .collect();
        if at_tier.is_empty() {
            return Err(MISSED);
        }
        at_tier
            .into_iter()
            .filter(|transcoding| !transcoding.is_preview(full_ms))
            .min_by_key(|transcoding| {
                (
                    transcoding.encrypted(),
                    transcoding.quality.as_deref() == Some(LOW),
                )
            })
            .map(|transcoding| transcoding.picked(tier))
            .ok_or(PREVIEW)
    }

    pub(super) fn lasts(&self, seconds: f64) -> Result<(), &'static str> {
        let full_ms = self.full_ms()?;
        if seconds * 1000.0 < full_ms as f64 * FULL_SHARE {
            return Err(PREVIEW);
        }
        Ok(())
    }

    fn full_ms(&self) -> Result<i64, &'static str> {
        self.full_duration.filter(|ms| *ms > 0).ok_or(PREVIEW)
    }
}

impl Authorization {
    fn opened_by(&self, plan: Option<&str>) -> bool {
        match plan.map(UserPlan::parse) {
            Some(UserPlan::GoPlus) => self.sub_high_tier == Some(true),
            Some(UserPlan::Go) => self.sub_mid_tier == Some(true),
            _ => false,
        }
    }
}

impl Transcoding {
    fn tier(&self) -> &'static str {
        quality::of_preset(
            self.preset.as_deref().unwrap_or(""),
            self.quality.as_deref(),
        )
    }

    fn protocol(&self) -> Option<&str> {
        self.format.as_ref()?.protocol.as_deref()
    }

    fn encrypted(&self) -> bool {
        self.protocol() == Some(ENCRYPTED)
    }

    fn streams(&self, decrypts: bool) -> bool {
        let adaptive = self
            .preset
            .as_deref()
            .is_some_and(|preset| preset.starts_with(ADAPTIVE));
        let readable = match self.protocol() {
            Some(CLEAR) => true,
            Some(ENCRYPTED) => decrypts,
            _ => false,
        };
        readable && !adaptive
    }

    fn is_preview(&self, full_ms: i64) -> bool {
        self.snipped == Some(true)
            || Url::parse(&self.url).is_ok_and(|url| names_a_preview(&url))
            || self
                .duration_ms
                .is_some_and(|ms| (ms as f64) < full_ms as f64 * FULL_SHARE)
    }

    fn picked(&self, tier: &'static str) -> Picked<'_> {
        Picked {
            url: &self.url,
            mime: self
                .format
                .as_ref()
                .and_then(|format| format.mime_type.as_deref())
                .unwrap_or(""),
            tier,
            encrypted: self.encrypted(),
        }
    }
}
