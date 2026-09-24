use std::time::Duration;

use backend_contracts::pipeline::{
    AI_RESOLVE_ARTIST, MAX_RESOLVE_DESCRIPTION_CHARS, RESOLVE_ARTIST_WINDOW_SECONDS,
    ResolveArtistData, ResolveArtistRequest, RpcSource,
};
use base64::Engine;
use catalog_sources::SourceError;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::debug;
use uuid::Uuid;

use crate::bus::Bus;
use crate::handlers::ai_store::AiStore;

use super::error::{EnrichError, EnrichResult};
use super::resolver::{
    AlbumCandidate, ArtistCandidate, CreditEvidence, ResolveResult, ResolveSource, TrackContext,
};

const LLM_ANSWER_TTL: i64 = 30 * 24 * 60 * 60;
const DETERMINISTIC_ANSWER_TTL: i64 = 24 * 60 * 60;
const MIN_TIMEOUT: Duration = Duration::from_secs(1);

pub struct AiResolverClient {
    bus: Bus,
    store: AiStore,
    timeout: Duration,
}

impl AiResolverClient {
    pub fn new(bus: Bus, pool: PgPool, timeout_ms: u64, daily_budget: u64) -> Self {
        Self {
            bus,
            store: AiStore::new(pool, daily_budget),
            timeout: resolve_timeout(timeout_ms),
        }
    }

    pub async fn resolve(&self, ctx: &TrackContext) -> EnrichResult<Option<ResolveResult>> {
        if ctx.title.trim().is_empty() {
            return Ok(None);
        }
        let request_hash = context_hash(ctx);
        let cache_key = format!("resolve:{request_hash}");
        if let Some(reply) = self.store.cached::<ResolveArtistData>(&cache_key).await {
            return Ok(build_result(reply, ctx));
        }
        if !self.store.take_budget().await {
            debug!("ai resolve daily budget exceeded");
            return Ok(None);
        }
        let message_id = format!("resolve_artist:{request_hash}:{}", Uuid::now_v7());
        let outcome = self
            .bus
            .request::<_, ResolveArtistData>(
                AI_RESOLVE_ARTIST,
                &resolve_request(ctx),
                self.timeout,
                &message_id,
            )
            .await;
        let reply = answered(outcome)?;
        self.store.settle_budget(reply.source).await;
        self.store
            .remember(&cache_key, &reply, answer_ttl(reply.source))
            .await;
        Ok(build_result(reply, ctx))
    }
}

fn answered(outcome: anyhow::Result<Option<ResolveArtistData>>) -> EnrichResult<ResolveArtistData> {
    match outcome {
        Ok(Some(reply)) => Ok(reply),
        Ok(None) => Err(ai_unavailable("the ai lane sent no answer")),
        Err(error) => {
            debug!(%error, "ai resolve request failed");
            Err(ai_unavailable(&format!("{error:#}")))
        }
    }
}

fn ai_unavailable(cause: &str) -> EnrichError {
    EnrichError::Source(SourceError::Unreachable(format!(
        "{AI_RESOLVE_ARTIST}: {cause}"
    )))
}

fn resolve_timeout(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms).clamp(
        MIN_TIMEOUT,
        Duration::from_secs(RESOLVE_ARTIST_WINDOW_SECONDS),
    )
}

fn resolve_request(ctx: &TrackContext) -> ResolveArtistRequest {
    ResolveArtistRequest {
        title: ctx.title.clone(),
        uploader: ctx.uploader_username.clone(),
        metadata_artist: ctx.metadata_artist.clone(),
        isrc: ctx.isrc.clone(),
        description: ctx.description.as_deref().map(|description| {
            description
                .chars()
                .take(MAX_RESOLVE_DESCRIPTION_CHARS as usize)
                .collect()
        }),
        duration_ms: ctx
            .duration_ms
            .filter(|duration_ms| *duration_ms >= 0)
            .map(i64::from),
    }
}

fn answer_ttl(source: RpcSource) -> i64 {
    match source {
        RpcSource::Llm => LLM_ANSWER_TTL,
        RpcSource::Deterministic => DETERMINISTIC_ANSWER_TTL,
    }
}

fn build_result(reply: ResolveArtistData, ctx: &TrackContext) -> Option<ResolveResult> {
    if reply.source == RpcSource::Deterministic {
        debug!("ai resolve answered without a language model; the local heuristic stands");
        return None;
    }
    let primary_name = reply
        .primary_artist
        .filter(|name| !name.trim().is_empty())?;
    let confidence = (reply.confidence as f32).clamp(0.3, 0.75);
    let candidate = |name: String| {
        ArtistCandidate {
            name,
            ..ArtistCandidate::default()
        }
        .attributed(ResolveSource::Ai, confidence, CreditEvidence::AiInference)
    };
    let named = |names: Vec<String>| {
        names
            .into_iter()
            .filter(|name| !name.trim().is_empty())
            .map(candidate)
            .collect()
    };

    Some(ResolveResult {
        source: ResolveSource::Ai,
        confidence,
        primary: vec![candidate(primary_name)],
        featured: named(reply.featured),
        producers: named(reply.producers),
        remixers: named(reply.remixers),
        album: reply.album.map(|album| AlbumCandidate {
            title: album.title,
            year: album.year.and_then(|year| i16::try_from(year).ok()),
            mb_id: None,
            genius_id: None,
            cover_url: None,
            release_type: None,
            primary_artist: album.primary_artist.map(candidate),
        }),
        isrc: ctx.isrc.clone(),
        ..Default::default()
    })
}

fn context_hash(ctx: &TrackContext) -> String {
    let mut hasher = Sha256::new();
    for part in [
        Some(ctx.title.as_str()),
        ctx.uploader_username.as_deref(),
        ctx.isrc.as_deref(),
        ctx.metadata_artist.as_deref(),
        ctx.description.as_deref(),
    ] {
        hasher.update(part.unwrap_or("").as_bytes());
        hasher.update(b"\x00");
    }
    if let Some(duration_ms) = ctx.duration_ms {
        hasher.update(duration_ms.to_le_bytes());
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
