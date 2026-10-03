use std::collections::HashSet;

use axum::Json;
use axum::http::{HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};
use catalog_match::evaluate_sc_candidate;
use catalog_normalize::normalize_title;
use serde::Serialize;
use serde_json::Value;

use super::meta::{LiveMeta, STATE_HEADER};
use super::slim::urn_of;
use crate::error::{AppError, AppResult};
use crate::modules::search::rank::version_penalty;
use crate::modules::search::terms::QueryTerms;

pub const ACCEPT_SCORE: f64 = 0.80;
const SHOWN_CANDIDATES: usize = 3;
const MAX_FIELD_CHARS: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct Wanted {
    pub artist: String,
    pub title: String,
    pub duration_ms: Option<i32>,
}

impl Wanted {
    pub fn parse(
        artist: Option<&str>,
        title: Option<&str>,
        duration_ms: Option<&str>,
    ) -> AppResult<Self> {
        let title = clipped(title);
        if normalize_title(&title).is_empty() {
            return Err(AppError::bad_request("title is required"));
        }
        Ok(Self {
            artist: clipped(artist),
            title,
            duration_ms: duration_ms
                .and_then(|raw| raw.trim().parse::<i32>().ok())
                .filter(|ms| *ms > 0),
        })
    }

    pub fn query(&self) -> String {
        format!("{} {}", self.artist, self.title).trim().to_owned()
    }
}

fn clipped(raw: Option<&str>) -> String {
    let kept: String = raw
        .unwrap_or_default()
        .trim()
        .chars()
        .take(MAX_FIELD_CHARS)
        .collect();
    kept.trim().to_owned()
}

#[derive(Clone, Debug, Serialize)]
pub struct Judged {
    pub urn: String,
    pub confidence: f64,
    pub source: &'static str,
    pub title: String,
    pub username: Option<String>,
    #[serde(skip)]
    pub playable: bool,
    #[serde(skip)]
    pub item: Value,
}

pub fn judge(
    wanted: &Wanted,
    item: Value,
    source: &'static str,
    indexed: Option<f32>,
) -> Option<Judged> {
    let urn = urn_of(&item)?.to_owned();
    let title = item.get("title").and_then(Value::as_str)?.to_owned();
    let mut found = evaluate_sc_candidate(
        &item,
        &wanted.title,
        &wanted.artist,
        None,
        wanted.duration_ms,
    );
    if let Some(score) = indexed {
        found.title_score = found.title_score.max(score);
        found.artist_score = 1.0;
    }
    let requested = QueryTerms::parse(&wanted.title).markers;
    let confidence = (found.score() - version_penalty(&requested, &title)).max(0.0);
    Some(Judged {
        urn,
        confidence: (f64::from(confidence) * 1000.0).round() / 1000.0,
        source,
        username: item
            .pointer("/user/username")
            .and_then(Value::as_str)
            .map(str::to_owned),
        playable: item.get("access").and_then(Value::as_str) != Some("preview"),
        title,
        item,
    })
}

#[derive(Debug, Serialize)]
pub struct MatchReply {
    #[serde(rename = "match")]
    pub found: Option<Judged>,
    pub candidates: Vec<Judged>,
    pub live: LiveMeta,
}

impl MatchReply {
    pub fn decide(mut judged: Vec<Judged>, live: LiveMeta) -> Self {
        judged.sort_by(|left, right| right.confidence.total_cmp(&left.confidence));
        let mut seen: HashSet<String> = HashSet::new();
        judged.retain(|one| seen.insert(one.urn.clone()));
        let found = judged
            .iter()
            .find(|one| one.playable && one.confidence >= ACCEPT_SCORE)
            .cloned();
        judged.truncate(SHOWN_CANDIDATES);
        Self {
            found,
            candidates: judged,
            live,
        }
    }
}

impl IntoResponse for MatchReply {
    fn into_response(self) -> Response {
        let state = self.live.state.as_str();
        let mut response = Json(self).into_response();
        response.headers_mut().insert(
            HeaderName::from_static(STATE_HEADER),
            HeaderValue::from_static(state),
        );
        response
    }
}
