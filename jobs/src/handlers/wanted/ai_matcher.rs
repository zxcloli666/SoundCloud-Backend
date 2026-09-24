use std::time::Duration;

use backend_contracts::pipeline::{
    self as contract, AI_MATCH_TRACK, MATCH_TRACK_WINDOW_SECONDS, MAX_MATCH_CANDIDATES,
    MatchTrackData, MatchTrackRequest, RpcSource,
};
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::debug;
use uuid::Uuid;

use crate::bus::Bus;
use crate::handlers::ai_store::AiStore;

const SETTLED_ANSWER_TTL: i64 = 30 * 24 * 60 * 60;
const UNSETTLED_ANSWER_TTL: i64 = 24 * 60 * 60;
const MIN_TIMEOUT: Duration = Duration::from_secs(1);

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

#[derive(Debug, Clone, Copy)]
pub struct AiMatch {
    pub candidate_id: u32,
    pub confidence: f32,
}

pub struct AiMatcherClient {
    bus: Bus,
    store: AiStore,
    timeout: Duration,
}

impl AiMatcherClient {
    pub fn new(bus: Bus, pool: PgPool, timeout_ms: u64, daily_budget: u64) -> Self {
        Self {
            bus,
            store: AiStore::new(pool, daily_budget),
            timeout: match_timeout(timeout_ms),
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
        let request_hash = request_hash(&request);
        let cache_key = format!("match:{request_hash}");
        if let Some(reply) = self.store.cached::<MatchTrackData>(&cache_key).await {
            return Ok(into_match(reply));
        }
        if !self.store.take_budget().await {
            debug!("ai match daily budget exceeded");
            return Err(AiUnavailable);
        }

        let message_id = format!("match_track:{request_hash}:{}", Uuid::now_v7());
        let outcome = self
            .bus
            .request::<_, MatchTrackData>(AI_MATCH_TRACK, &request, self.timeout, &message_id)
            .await;
        let reply = answered(outcome)?;
        self.store.settle_budget(reply.source).await;
        self.store
            .remember(&cache_key, &reply, answer_ttl(&reply))
            .await;
        Ok(into_match(reply))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiUnavailable;

fn answered(
    outcome: anyhow::Result<Option<MatchTrackData>>,
) -> Result<MatchTrackData, AiUnavailable> {
    match outcome {
        Ok(Some(reply)) => Ok(reply),
        Ok(None) => {
            debug!("ai match lane sent no answer");
            Err(AiUnavailable)
        }
        Err(error) => {
            debug!(%error, "ai match request failed");
            Err(AiUnavailable)
        }
    }
}

fn match_timeout(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms)
        .clamp(MIN_TIMEOUT, Duration::from_secs(MATCH_TRACK_WINDOW_SECONDS))
}

fn match_request(target: &MatchTarget, candidates: &[MatchCandidate]) -> MatchTrackRequest {
    MatchTrackRequest {
        target: contract::MatchTarget {
            artist: target.artist.to_owned(),
            title: target.title.to_owned(),
        },
        candidates: candidates
            .iter()
            .take(MAX_MATCH_CANDIDATES as usize)
            .map(|candidate| contract::MatchCandidate {
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

fn answer_ttl(reply: &MatchTrackData) -> i64 {
    if reply.source == RpcSource::Llm || reply.match_id.is_some() {
        SETTLED_ANSWER_TTL
    } else {
        UNSETTLED_ANSWER_TTL
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

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: u32) -> MatchCandidate<'static> {
        MatchCandidate {
            id,
            artist: "Artist",
            title: "Song",
            duration_sec: Some(-5),
        }
    }

    fn reply(body: &str) -> anyhow::Result<MatchTrackData> {
        Ok(serde_json::from_str(body)?)
    }

    #[test]
    fn the_request_stays_inside_the_contract_limits() -> anyhow::Result<()> {
        let candidates = (0..60).map(candidate).collect::<Vec<_>>();
        let target = MatchTarget {
            artist: "Artist",
            title: "Song",
        };

        let request = match_request(&target, &candidates);
        let encoded = serde_json::to_value(&request)?;

        assert_eq!(request.candidates.len(), MAX_MATCH_CANDIDATES as usize);
        assert_eq!(encoded["candidates"][0]["duration_sec"], Value::Null);
        assert_eq!(encoded["target"]["title"], "Song");
        Ok(())
    }

    #[test]
    fn a_tie_is_no_match_and_is_asked_again_tomorrow() -> anyhow::Result<()> {
        let tie = reply(r#"{"match_id":null,"confidence":0.62,"source":"deterministic"}"#)?;

        assert_eq!(answer_ttl(&tie), UNSETTLED_ANSWER_TTL);
        assert!(into_match(tie).is_none());
        Ok(())
    }

    #[test]
    fn a_confident_match_is_kept_whatever_answered_it() -> anyhow::Result<()> {
        let deterministic = reply(r#"{"match_id":3,"confidence":0.99,"source":"deterministic"}"#)?;
        let llm = reply(r#"{"match_id":null,"confidence":0.4,"source":"llm"}"#)?;

        assert_eq!(answer_ttl(&deterministic), SETTLED_ANSWER_TTL);
        assert_eq!(answer_ttl(&llm), SETTLED_ANSWER_TTL);
        let picked = into_match(deterministic).ok_or_else(|| anyhow::anyhow!("no match"))?;
        assert_eq!(picked.candidate_id, 3);
        assert_eq!(picked.confidence, 0.95);
        Ok(())
    }

    #[test]
    fn a_reply_without_its_source_is_not_read() {
        assert!(
            serde_json::from_str::<MatchTrackData>(r#"{"match_id":1,"confidence":0.9}"#).is_err()
        );
    }

    #[test]
    fn a_silent_or_failing_ai_lane_is_not_a_no_match_answer() -> anyhow::Result<()> {
        let answer = answered(Ok(Some(reply(
            r#"{"match_id":null,"confidence":0.4,"source":"llm"}"#,
        )?)));

        assert_eq!(answered(Ok(None)).err(), Some(AiUnavailable));
        assert_eq!(
            answered(Err(anyhow::anyhow!("NATS request timed out"))).err(),
            Some(AiUnavailable)
        );
        assert!(answer.is_ok_and(|reply| into_match(reply).is_none()));
        Ok(())
    }

    #[test]
    fn a_match_waits_no_longer_than_the_contract_window() {
        assert_eq!(match_timeout(20_000), Duration::from_secs(10));
        assert_eq!(match_timeout(4_000), Duration::from_secs(4));
    }
}
