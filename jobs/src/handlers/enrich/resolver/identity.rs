use sqlx::PgPool;
use tracing::debug;

use crate::handlers::enrich::error::EnrichResult;
use catalog_normalize::{name_in, same_artist};

use super::merge::enrich_with_local_signals;
use super::signals::LocalSignals;
use super::{ArtistCandidate, CreditEvidence, ResolveResult, ResolveSource, TrackContext};

const MANUAL_IDENTITY_CONFIDENCE: f32 = 0.9;
const EXTERNAL_IDENTITY_CONFIDENCE: f32 = 0.75;

pub(super) struct ScIdentity {
    artist: ArtistCandidate,
    manual: bool,
}

pub(super) async fn lookup(ctx: &TrackContext, pg: &PgPool) -> EnrichResult<Option<ScIdentity>> {
    let Some(sc_user_id) = ctx
        .uploader_sc_user_id
        .as_deref()
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let claims = sqlx::query_file!("queries/enrich/service/sc_identity_artist.sql", sc_user_id)
        .fetch_all(pg)
        .await?;
    let Some(claim) = claims.first() else {
        return Ok(None);
    };
    if claims.len() > 1 && !claim.verified {
        debug!(
            sc_user_id,
            "sc identity is ambiguous, no artist claims the upload"
        );
        return Ok(None);
    }
    Ok(Some(ScIdentity {
        artist: ArtistCandidate {
            name: claim.name.clone(),
            mb_id: claim.mb_artist_id.clone(),
            genius_id: claim.genius_artist_id.clone(),
            sc_user_id: Some(sc_user_id.to_owned()),
            ..Default::default()
        }
        .attributed(
            ResolveSource::ScVerified,
            if claim.verified { 0.95 } else { 0.7 },
            CreditEvidence::VerifiedAccount,
        ),
        manual: claim.verified,
    }))
}

pub(super) fn reconcile(
    result: ResolveResult,
    identity: ScIdentity,
    ctx: &TrackContext,
    signals: &LocalSignals,
) -> ResolveResult {
    if result.degraded {
        return result;
    }
    if !matches!(result.source, ResolveSource::Heuristic) {
        if credits_identity(&result, &identity) {
            return attach_identity(result, &identity);
        }
        return result;
    }
    if let Err(reason) = claim_gate(&identity.artist.name, signals) {
        debug!(artist = %identity.artist.name, reason, "sc identity claim skipped");
        return result;
    }
    claim(identity, ctx, signals)
}

fn credits_identity(result: &ResolveResult, identity: &ScIdentity) -> bool {
    result.primary.iter().any(|candidate| {
        same_external_id(candidate.mb_id.as_deref(), identity.artist.mb_id.as_deref())
            || same_external_id(
                candidate.genius_id.as_deref(),
                identity.artist.genius_id.as_deref(),
            )
            || same_artist(&candidate.name, &identity.artist.name)
    })
}

fn same_external_id(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left == right)
}

fn attach_identity(mut result: ResolveResult, identity: &ScIdentity) -> ResolveResult {
    for candidate in result.primary.iter_mut() {
        if !same_artist(&candidate.name, &identity.artist.name) {
            continue;
        }
        if candidate.sc_user_id.is_none() {
            candidate.sc_user_id = identity.artist.sc_user_id.clone();
        }
        if candidate.mb_id.is_none() {
            candidate.mb_id = identity.artist.mb_id.clone();
        }
        if candidate.genius_id.is_none() {
            candidate.genius_id = identity.artist.genius_id.clone();
        }
    }
    result
}

fn claim(identity: ScIdentity, ctx: &TrackContext, signals: &LocalSignals) -> ResolveResult {
    let confidence = if identity.manual {
        MANUAL_IDENTITY_CONFIDENCE
    } else {
        EXTERNAL_IDENTITY_CONFIDENCE
    };
    let claimed = ResolveResult {
        source: ResolveSource::ScVerified,
        confidence,
        primary: vec![identity.artist],
        isrc: ctx.isrc.clone(),
        ..Default::default()
    };
    enrich_with_local_signals(claimed, ctx, signals)
}

fn claim_gate(artist_name: &str, signals: &LocalSignals) -> Result<(), &'static str> {
    if signals.parsed.primary_from_title {
        let title_claims_other = signals
            .parsed
            .primary_artists
            .first()
            .map(|primary| !same_artist(primary, artist_name))
            .unwrap_or(false);
        if title_claims_other {
            return Err("title claims different artist");
        }
    }
    if !signals.meta_names.is_empty()
        && !name_in(artist_name, signals.meta_names.iter().map(|s| s.as_str()))
    {
        return Err("metadata names other artists");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{ctx, run_heuristic, signals_no_dict};
    use super::*;

    fn identity(name: &str, manual: bool) -> ScIdentity {
        ScIdentity {
            artist: ArtistCandidate {
                name: name.to_owned(),
                mb_id: None,
                genius_id: None,
                sc_user_id: Some("42".to_owned()),
                ..Default::default()
            },
            manual,
        }
    }

    fn external(source: ResolveSource, primary: &str) -> ResolveResult {
        ResolveResult {
            source,
            confidence: 0.8,
            primary: vec![ArtistCandidate {
                name: primary.to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn reconcile_for(
        title: &str,
        uploader: Option<&str>,
        meta: Option<&str>,
        result: ResolveResult,
        identity: ScIdentity,
    ) -> ResolveResult {
        let context = ctx(title, uploader, meta);
        let signals = signals_no_dict(&context);
        reconcile(result, identity, &context, &signals)
    }

    #[test]
    fn foreign_genius_credit_stays_a_reupload() {
        let out = reconcile_for(
            "Drake - God's Plan",
            Some("Zemix"),
            None,
            external(ResolveSource::Genius, "Drake"),
            identity("Zemix", true),
        );

        assert_eq!(out.source, ResolveSource::Genius);
        assert_eq!(out.primary.first().map(|c| c.name.as_str()), Some("Drake"));
        assert!(out.primary.iter().all(|c| c.sc_user_id.is_none()));
    }

    #[test]
    fn agreeing_genius_credit_takes_the_account_id() {
        let out = reconcile_for(
            "МОКЕРИ - kill",
            Some("МОКЕРИ"),
            None,
            external(ResolveSource::Genius, "МОКЕРИ"),
            identity("МОКЕРИ", true),
        );

        assert_eq!(out.source, ResolveSource::Genius);
        assert_eq!(
            out.primary.first().and_then(|c| c.sc_user_id.as_deref()),
            Some("42")
        );
    }

    #[test]
    fn genius_miss_lets_a_verified_account_claim_its_own_upload() {
        let context = ctx("без шансов", Some("dekma"), None);
        let signals = signals_no_dict(&context);
        let out = reconcile(
            run_heuristic(&context),
            identity("dekma", true),
            &context,
            &signals,
        );

        assert_eq!(out.source, ResolveSource::ScVerified);
        assert_eq!(out.confidence, MANUAL_IDENTITY_CONFIDENCE);
        assert_eq!(out.primary.first().map(|c| c.name.as_str()), Some("dekma"));
    }

    #[test]
    fn external_account_link_claims_with_lower_confidence() {
        let context = ctx("без шансов", Some("dekma"), None);
        let signals = signals_no_dict(&context);
        let out = reconcile(
            run_heuristic(&context),
            identity("dekma", false),
            &context,
            &signals,
        );

        assert_eq!(out.source, ResolveSource::ScVerified);
        assert_eq!(out.confidence, EXTERNAL_IDENTITY_CONFIDENCE);
    }

    #[test]
    fn foreign_metadata_blocks_the_account_claim() {
        let context = ctx(
            "DISTORTED DREAMS",
            Some("Zemix"),
            Some("frxchtzwxrg & m∞nflower"),
        );
        let signals = signals_no_dict(&context);
        let out = reconcile(
            run_heuristic(&context),
            identity("Zemix", true),
            &context,
            &signals,
        );

        assert!(!matches!(out.source, ResolveSource::ScVerified));
    }

    #[test]
    fn transient_source_failure_never_hands_the_upload_to_the_account() {
        let context = ctx("без шансов", Some("dekma"), None);
        let signals = signals_no_dict(&context);
        let mut degraded = run_heuristic(&context);
        degraded.degraded = true;
        let out = reconcile(degraded, identity("dekma", true), &context, &signals);

        assert!(!matches!(out.source, ResolveSource::ScVerified));
    }
}
