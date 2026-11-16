mod prompt;
mod rank;
mod request;
mod similarity;

#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;
#[cfg(test)]
#[path = "rank_tests.rs"]
mod rank_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use llm_ladder::Ladder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::debug;

use self::prompt::Pick;
use self::rank::{Scored, decide, is_borderline, rank};
use self::request::MatchTrackRequest;
use crate::config::AiConfig;
use crate::handlers::ai_store::{AiStore, RpcSource};
use crate::handlers::metadata::Normalizer;
use crate::llm::{AiMethod, AiOutcome, record_answer};

const LLM_CANDIDATES: usize = 10;
const MIN_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_MATCH_CANDIDATES: u32 = 50;
const MATCH_TRACK_WINDOW_SECONDS: u64 = 10;

#[derive(Debug, Clone)]
pub struct MatchTarget<'a> {
    pub artist: &'a str,
    pub title: &'a str,
}

#[derive(Debug, Clone)]
pub struct MatchCandidate<'a> {
    pub id: u32,
    pub artist: &'a str,
    pub title: &'a str,
    pub duration_sec: Option<i32>,
}

impl<'a> MatchCandidate<'a> {
    pub fn from_sc(id: u32, track: &'a Value) -> Self {
        Self {
            id,
            artist: track
                .get("user")
                .and_then(|user| user.get("username"))
                .and_then(Value::as_str)
                .unwrap_or(""),
            title: track.get("title").and_then(Value::as_str).unwrap_or(""),
            duration_sec: track
                .get("duration")
                .and_then(Value::as_i64)
                .map(|milliseconds| (milliseconds / 1000) as i32),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MatchTrackData {
    pub match_id: Option<u32>,
    pub confidence: f64,
    pub source: RpcSource,
}

#[derive(Debug, Clone, Copy)]
pub struct AiMatch {
    pub candidate_id: u32,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiUnavailable;

pub struct AiMatcherClient {
    ladder: Arc<Ladder>,
    normalizer: Arc<Normalizer>,
    store: AiStore,
    timeout: Duration,
}

impl AiMatcherClient {
    pub fn new(
        ladder: Arc<Ladder>,
        normalizer: Arc<Normalizer>,
        pool: PgPool,
        config: &AiConfig,
    ) -> Self {
        Self {
            ladder,
            normalizer,
            store: AiStore::new(pool, config.daily_budget),
            timeout: match_timeout(config.timeout_ms),
        }
    }

    pub async fn pick(
        &self,
        target: MatchTarget<'_>,
        candidates: &[MatchCandidate<'_>],
    ) -> Result<Option<AiMatch>, AiUnavailable> {
        if candidates.is_empty() || target.title.trim().is_empty() {
            return Ok(None);
        }
        let request = match_request(&target, candidates);
        let ranked = rank(&self.normalizer, &request.target, &request.candidates);
        let verdict = decide(&ranked);
        if !is_borderline(&ranked) {
            record_answer(AiMethod::MatchTrack, AiOutcome::Confident);
            return Ok(into_match(verdict));
        }
        let cache_key = format!("match:{}", request_hash(&request));
        if let Some(reply) = self.store.cached::<MatchTrackData>(&cache_key).await {
            record_answer(AiMethod::MatchTrack, AiOutcome::Cached);
            return Ok(into_match(reply));
        }
        if !self.store.take_budget().await {
            debug!("ai match daily budget exceeded");
            record_answer(AiMethod::MatchTrack, AiOutcome::OverBudget);
            return Err(AiUnavailable);
        }
        let shortlist = ranked.get(..LLM_CANDIDATES).unwrap_or(&ranked);
        match self.ask(&request.target, shortlist).await {
            Some(answer) => {
                self.store.remember(&cache_key, &answer).await;
                record_answer(AiMethod::MatchTrack, AiOutcome::Llm);
                Ok(into_match(answer))
            }
            None => {
                self.store.refund_budget().await;
                record_answer(AiMethod::MatchTrack, AiOutcome::Fallback);
                Ok(into_match(verdict))
            }
        }
    }

    async fn ask(
        &self,
        target: &request::MatchTarget,
        shortlist: &[Scored<'_>],
    ) -> Option<MatchTrackData> {
        let allowed: HashSet<u32> = shortlist.iter().map(|scored| scored.candidate.id).collect();
        let checked = allowed.clone();
        let asked = self
            .ladder
            .complete_if(
                prompt::request(target, shortlist, self.timeout),
                move |pick: &Pick| prompt::read(pick, &checked).is_some(),
            )
            .await;
        match asked {
            Ok(completion) => prompt::read(&completion.value, &allowed),
            Err(error) => {
                debug!(%error, "the ladder gave no match answer");
                None
            }
        }
    }
}

fn match_timeout(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms)
        .clamp(MIN_TIMEOUT, Duration::from_secs(MATCH_TRACK_WINDOW_SECONDS))
}

fn match_request(target: &MatchTarget, candidates: &[MatchCandidate]) -> MatchTrackRequest {
    MatchTrackRequest {
        target: request::MatchTarget {
            artist: target.artist.to_owned(),
            title: target.title.to_owned(),
        },
        candidates: candidates
            .iter()
            .take(MAX_MATCH_CANDIDATES as usize)
            .map(|candidate| request::MatchCandidate {
                id: candidate.id,
                artist: candidate.artist.to_owned(),
                title: candidate.title.to_owned(),
                duration_sec: candidate
                    .duration_sec
                    .filter(|seconds| *seconds >= 0)
                    .map(f64::from),
            })
            .collect(),
    }
}

fn into_match(reply: MatchTrackData) -> Option<AiMatch> {
    Some(AiMatch {
        candidate_id: reply.match_id?,
        confidence: (reply.confidence as f32).clamp(0.3, 0.95),
    })
}

fn request_hash(request: &MatchTrackRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.target.artist.as_bytes());
    hasher.update(b"\x00");
    hasher.update(request.target.title.as_bytes());
    hasher.update(b"\x00");
    for candidate in &request.candidates {
        hasher.update(candidate.id.to_le_bytes());
        hasher.update(candidate.artist.as_bytes());
        hasher.update(b"\x00");
        hasher.update(candidate.title.as_bytes());
        hasher.update(b"\x00");
        if let Some(duration_sec) = candidate.duration_sec {
            hasher.update(duration_sec.to_le_bytes());
        }
        hasher.update(b"\x01");
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}
