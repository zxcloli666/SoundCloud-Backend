mod answer;
mod guess;
mod prompt;

#[cfg(test)]
#[path = "answer_tests.rs"]
mod answer_tests;
#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;
#[cfg(test)]
#[path = "guess_tests.rs"]
mod guess_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::pipeline::{
    MAX_RESOLVE_DESCRIPTION_CHARS, RESOLVE_ARTIST_WINDOW_SECONDS, ResolveArtistData,
    ResolveArtistRequest, RpcSource,
};
use base64::Engine;
use llm_ladder::Ladder;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::debug;

use self::answer::{Grounding, ModelAnswer};
use self::guess::{Guess, deterministic};
use super::resolver::{
    AlbumCandidate, ArtistCandidate, CreditEvidence, ResolveResult, ResolveSource, TrackContext,
};
use crate::config::AiConfig;
use crate::handlers::ai_store::AiStore;
use crate::handlers::metadata::Normalizer;
use crate::llm::{AiMethod, AiOutcome, record_answer};

const MIN_TIMEOUT: Duration = Duration::from_secs(1);

pub struct AiResolverClient {
    ladder: Arc<Ladder>,
    normalizer: Arc<Normalizer>,
    store: AiStore,
    timeout: Duration,
}

impl AiResolverClient {
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
            timeout: resolve_timeout(config.timeout_ms),
        }
    }

    pub async fn resolve(&self, ctx: &TrackContext) -> Option<ResolveResult> {
        if ctx.title.trim().is_empty() {
            return None;
        }
        let track = resolve_request(ctx);
        let guess = deterministic(&self.normalizer, &track);
        if guess.is_confident() {
            record_answer(AiMethod::ResolveArtist, AiOutcome::Confident);
            return None;
        }
        let cache_key = format!("resolve:{}", context_hash(ctx));
        if let Some(reply) = self.store.cached::<ResolveArtistData>(&cache_key).await {
            record_answer(AiMethod::ResolveArtist, AiOutcome::Cached);
            return build_result(reply, ctx);
        }
        if !self.store.take_budget().await {
            debug!("ai resolve daily budget exceeded");
            record_answer(AiMethod::ResolveArtist, AiOutcome::OverBudget);
            return None;
        }
        match self.ask(&track, &guess).await {
            Some(answer) => {
                self.store.remember(&cache_key, &answer).await;
                record_answer(AiMethod::ResolveArtist, AiOutcome::Llm);
                build_result(answer, ctx)
            }
            None => {
                self.store.refund_budget().await;
                record_answer(AiMethod::ResolveArtist, AiOutcome::Fallback);
                None
            }
        }
    }

    async fn ask(&self, track: &ResolveArtistRequest, guess: &Guess) -> Option<ResolveArtistData> {
        let grounding = Grounding::new(track, guess.primary_artist.is_some());
        let checked = grounding.clone();
        let asked = self
            .ladder
            .complete_if(
                prompt::request(track, guess, self.timeout),
                move |reply: &ModelAnswer| checked.answer(reply).is_some(),
            )
            .await;
        match asked {
            Ok(completion) => grounding.answer(&completion.value),
            Err(error) => {
                debug!(%error, "the ladder gave no grounded artist answer");
                None
            }
        }
    }
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

fn build_result(reply: ResolveArtistData, ctx: &TrackContext) -> Option<ResolveResult> {
    if reply.source == RpcSource::Deterministic {
        debug!("a cached deterministic answer leaves the local heuristic in charge");
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
