mod identity;
mod mb_stage;
mod merge;
mod signals;

use std::sync::Arc;

use tracing::{debug, warn};

use crate::handlers::enrich::ai::AiResolverClient;
use crate::handlers::enrich::error::EnrichResult;
use crate::handlers::enrich::genius_stage;
use catalog_normalize::normalize_name;
use catalog_sources::{GeniusService, MbClient};

pub use signals::LocalSignals;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResolveSource {
    #[default]
    Heuristic,
    Meta,
    Ai,
    Genius,
    Mb,
    Isrc,
    ScVerified,
}

impl ResolveSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::Meta => "meta",
            Self::Ai => "ai",
            Self::Genius => "genius",
            Self::Mb => "mb",
            Self::Isrc => "isrc",
            Self::ScVerified => "sc_verified",
        }
    }
    pub fn priority(&self) -> u8 {
        match self {
            Self::Heuristic => 1,
            Self::Meta => 2,
            Self::Ai => 3,
            Self::Genius => 4,
            Self::Mb => 5,
            Self::Isrc => 6,
            Self::ScVerified => 7,
        }
    }
    pub fn from_db(s: &str) -> Self {
        match s {
            "sc_verified" => Self::ScVerified,
            "isrc" => Self::Isrc,
            "mb" => Self::Mb,
            "genius" => Self::Genius,
            "ai" => Self::Ai,
            "meta" => Self::Meta,
            _ => Self::Heuristic,
        }
    }
    pub fn priority_of(s: &str) -> u8 {
        Self::from_db(s).priority()
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum CreditEvidence {
    #[default]
    Unattributed,
    ExternalId,
    VerifiedAccount,
    ExternalCredit,
    MetadataField,
    TitleHeuristic,
    AiInference,
    UploaderName,
}

impl CreditEvidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unattributed => "unattributed",
            Self::ExternalId => "external_id",
            Self::VerifiedAccount => "verified_account",
            Self::ExternalCredit => "external_credit",
            Self::MetadataField => "metadata_field",
            Self::TitleHeuristic => "title_heuristic",
            Self::AiInference => "ai_inference",
            Self::UploaderName => "uploader_name",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ArtistCandidate {
    pub name: String,
    pub mb_id: Option<String>,
    pub genius_id: Option<String>,
    pub sc_user_id: Option<String>,
    pub source: ResolveSource,
    pub confidence: f32,
    pub evidence: CreditEvidence,
}

impl ArtistCandidate {
    pub fn attributed(
        mut self,
        source: ResolveSource,
        confidence: f32,
        evidence: CreditEvidence,
    ) -> Self {
        self.source = source;
        self.confidence = confidence.clamp(0.0, 1.0);
        self.evidence = evidence;
        self
    }

    pub fn credit(&self) -> Credit {
        Credit {
            source: self.source,
            confidence: self.confidence.clamp(0.0, 1.0),
            evidence: self.evidence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Credit {
    pub source: ResolveSource,
    pub confidence: f32,
    pub evidence: CreditEvidence,
}

#[derive(Debug, Clone)]
pub struct AlbumCandidate {
    pub title: String,
    pub year: Option<i16>,
    pub mb_id: Option<String>,
    pub genius_id: Option<String>,
    pub cover_url: Option<String>,
    pub release_type: Option<String>,
    pub primary_artist: Option<ArtistCandidate>,
}

#[derive(Debug, Clone, Default)]
pub struct ResolveResult {
    pub source: ResolveSource,
    pub confidence: f32,
    pub primary: Vec<ArtistCandidate>,
    pub featured: Vec<ArtistCandidate>,
    pub producers: Vec<ArtistCandidate>,
    pub remixers: Vec<ArtistCandidate>,
    pub album: Option<AlbumCandidate>,
    pub isrc: Option<String>,
    pub release_date: Option<chrono::NaiveDate>,
    pub release_year: Option<i16>,
    pub is_cover: bool,
    pub degraded: bool,
    pub genius_song_id: Option<i64>,
    pub genius_url: Option<String>,
}

pub struct TrackContext {
    pub title: String,
    pub uploader_username: Option<String>,
    pub uploader_sc_user_id: Option<String>,
    pub duration_ms: Option<i32>,
    pub isrc: Option<String>,
    pub metadata_artist: Option<String>,
    pub description: Option<String>,
}

pub struct ResolverDeps {
    pub mb: Arc<MbClient>,
    pub genius: Arc<GeniusService>,
    pub ai: Option<Arc<AiResolverClient>>,
    pub pg: sqlx::PgPool,
}

pub async fn resolve_track(ctx: &TrackContext, deps: &ResolverDeps) -> EnrichResult<ResolveResult> {
    let signals = LocalSignals::build(ctx, &deps.pg).await;
    let identity = identity::lookup(ctx, &deps.pg).await?;

    let result = resolve(ctx, &signals, deps).await?;
    Ok(match identity {
        Some(identity) => identity::reconcile(result, identity, ctx, &signals),
        None => result,
    })
}

async fn resolve(
    ctx: &TrackContext,
    signals: &LocalSignals,
    deps: &ResolverDeps,
) -> EnrichResult<ResolveResult> {
    let heuristic = signals.heuristic(ctx);
    let mut degraded = false;

    if let Some(isrc) = ctx.isrc.as_ref() {
        match deps.mb.lookup_by_isrc(isrc).await {
            Ok(Some(rec)) => {
                let ext = mb_stage::from_mb(rec, ResolveSource::Isrc, 0.95, Some(isrc.clone()));
                return Ok(merge::merge_with(heuristic, ext, ctx, signals));
            }
            Ok(None) => debug!(isrc, "ISRC lookup empty"),
            Err(e) => {
                debug!(error = %e, isrc, "ISRC lookup failed");
                degraded = true;
            }
        }
    }

    let primary_hint = signals.primary_hint(ctx);
    let title_q = signals.title_query(ctx);

    if let Some(ext) = mb_stage::search(ctx, signals, deps, &primary_hint, &title_q).await {
        let mut out = merge::merge_with(heuristic, ext, ctx, signals);
        out.degraded = degraded;
        return Ok(out);
    }

    match genius_stage::search(&deps.genius, ctx, primary_hint.as_deref(), &title_q).await {
        Ok(Some(res)) => {
            let mut out = merge::merge_with(heuristic, res, ctx, signals);
            out.degraded = degraded;
            return Ok(out);
        }
        Ok(None) => debug!(title_q, "Genius search empty"),
        Err(e) => {
            warn!(error = %e, "Genius search failed");
            degraded = true;
        }
    }

    if let Some(meta_a) = signals.meta_names.first().map(|s| s.as_str()) {
        let differs = primary_hint
            .as_deref()
            .map(|h| normalize_name(meta_a) != normalize_name(h))
            .unwrap_or(true);
        if differs {
            match genius_stage::search(&deps.genius, ctx, Some(meta_a), &title_q).await {
                Ok(Some(res)) => {
                    let mut out = merge::merge_with(heuristic, res, ctx, signals);
                    out.degraded = degraded;
                    return Ok(out);
                }
                Ok(None) => debug!(meta_a, "Genius search empty (metadata_artist)"),
                Err(e) => {
                    warn!(error = %e, "Genius search failed (metadata_artist)");
                    degraded = true;
                }
            }
        }
    }

    if let Some(ai) = deps.ai.as_ref() {
        match ai.resolve(ctx).await {
            Ok(Some(res)) => {
                let mut out = merge::merge_with(heuristic, res, ctx, signals);
                out.degraded = degraded;
                return Ok(out);
            }
            Ok(None) => debug!("AI resolve empty"),
            Err(e) => {
                debug!(error = %e, "AI resolve failed");
                degraded = true;
            }
        }
    }

    let mut out = heuristic;
    out.degraded = degraded;
    Ok(out)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn ctx(title: &str, uploader: Option<&str>, meta: Option<&str>) -> TrackContext {
        TrackContext {
            title: title.to_string(),
            uploader_username: uploader.map(String::from),
            uploader_sc_user_id: uploader.map(|_| "42".to_string()),
            duration_ms: Some(180_000),
            isrc: None,
            metadata_artist: meta.map(String::from),
            description: None,
        }
    }

    pub fn signals_no_dict(c: &TrackContext) -> LocalSignals {
        LocalSignals::build_with_dictionary(c, &std::collections::HashSet::new())
    }

    pub fn run_heuristic(c: &TrackContext) -> ResolveResult {
        signals_no_dict(c).heuristic(c)
    }

    pub fn names(r: &ResolveResult) -> Vec<&str> {
        r.primary.iter().map(|c| c.name.as_str()).collect()
    }
}

#[cfg(test)]
mod credit_tests {
    use super::*;

    fn bare(name: &str) -> ArtistCandidate {
        ArtistCandidate {
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_candidate_carries_its_own_provenance() {
        let credit = bare("mb artist")
            .attributed(ResolveSource::Mb, 0.9, CreditEvidence::ExternalId)
            .credit();

        assert_eq!(credit.source, ResolveSource::Mb);
        assert_eq!(credit.confidence, 0.9);
        assert_eq!(credit.evidence, CreditEvidence::ExternalId);
    }

    #[test]
    fn an_unmarked_candidate_is_the_weakest_possible_credit() {
        let credit = bare("nameless").credit();

        assert_eq!(credit.source, ResolveSource::Heuristic);
        assert_eq!(credit.confidence, 0.0);
        assert_eq!(credit.evidence, CreditEvidence::Unattributed);
    }

    #[test]
    fn confidence_is_clamped_into_the_unit_range() {
        let high = bare("a").attributed(ResolveSource::Mb, 4.2, CreditEvidence::ExternalId);
        let low = bare("b").attributed(ResolveSource::Ai, -1.0, CreditEvidence::AiInference);

        assert_eq!(high.confidence, 1.0);
        assert_eq!(low.confidence, 0.0);
    }

    #[test]
    fn every_evidence_code_is_distinct_and_survives_the_database_check() {
        let all = [
            CreditEvidence::Unattributed,
            CreditEvidence::ExternalId,
            CreditEvidence::VerifiedAccount,
            CreditEvidence::ExternalCredit,
            CreditEvidence::MetadataField,
            CreditEvidence::TitleHeuristic,
            CreditEvidence::AiInference,
            CreditEvidence::UploaderName,
        ];
        let names: std::collections::HashSet<&str> =
            all.iter().map(|evidence| evidence.as_str()).collect();

        assert_eq!(names.len(), all.len());
        assert!(all.iter().all(|evidence| evidence.as_str().len() <= 24));
    }
}
