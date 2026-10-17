use sqlx::{Postgres, Transaction};
use tracing::debug;
use uuid::Uuid;

use crate::handlers::enrich::error::EnrichResult;
use crate::handlers::enrich::resolver::{ArtistCandidate, CreditEvidence, ResolveSource};
use catalog_normalize::{clean_artist_name, normalize_name};

const MIN_NAME_MATCH_CHARS: usize = 4;
const REUPLOAD_THRESHOLD: i64 = 3;

pub(super) async fn without_foreign_namesakes(
    tx: &mut Transaction<'_, Postgres>,
    candidates: &[ArtistCandidate],
    uploader_sc_user_id: Option<&str>,
    uploader_username: Option<&str>,
) -> EnrichResult<Vec<ArtistCandidate>> {
    let uploader_name = uploader_username.map(normalize_name);
    let mut kept = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let name_only = candidate.mb_id.is_none()
            && candidate.genius_id.is_none()
            && !matches!(
                candidate.evidence,
                CreditEvidence::ExternalId
                    | CreditEvidence::ExternalCredit
                    | CreditEvidence::VerifiedAccount
            );
        let normalized = normalize_name(&clean_artist_name(&candidate.name));
        let ambiguous = normalized.chars().count() < MIN_NAME_MATCH_CHARS
            || uploader_name.as_deref() == Some(normalized.as_str());
        let held_by_other_accounts = name_only
            && ambiguous
            && sqlx::query_file_scalar!(
                "queries/enrich/persist/name_held_by_other_accounts.sql",
                normalized,
                uploader_sc_user_id
            )
            .fetch_optional(&mut **tx)
            .await?
            .unwrap_or(false);
        if held_by_other_accounts {
            debug!(
                artist = %candidate.name,
                uploader = ?uploader_sc_user_id,
                "name-only primary credit left to the artist's own accounts"
            );
            continue;
        }
        kept.push(candidate.clone());
    }
    Ok(kept)
}

pub(super) async fn maybe_auto_attach_sc_account(
    tx: &mut Transaction<'_, Postgres>,
    artist_id: Uuid,
    sc_user_id: &str,
    uploader_username: &str,
    artist_name: &str,
) -> EnrichResult<()> {
    let uploader = normalize_name(uploader_username);
    let artist = normalize_name(artist_name);
    let long_enough = uploader.chars().count() >= MIN_NAME_MATCH_CHARS
        && artist.chars().count() >= MIN_NAME_MATCH_CHARS;
    let names_agree = uploader.contains(&artist) || artist.contains(&uploader);
    if sc_user_id.is_empty() || !long_enough || !names_agree {
        return Ok(());
    }
    sqlx::query_file!(
        "queries/enrich/persist/insert_auto_match_account.sql",
        artist_id,
        sc_user_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn maybe_attach_reupload_account(
    tx: &mut Transaction<'_, Postgres>,
    artist_id: Uuid,
    sc_user_id: &str,
) -> EnrichResult<()> {
    if sc_user_id.is_empty() {
        return Ok(());
    }
    let exists: Option<i32> = sqlx::query_file_scalar!(
        "queries/enrich/persist/sc_account_exists.sql",
        artist_id,
        sc_user_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    if exists.is_some() {
        return Ok(());
    }
    let count: i64 = sqlx::query_file_scalar!(
        "queries/enrich/persist/count_reupload_tracks.sql",
        sc_user_id,
        artist_id
    )
    .fetch_one(&mut **tx)
    .await?;
    if count < REUPLOAD_THRESHOLD {
        return Ok(());
    }
    sqlx::query_file!(
        "queries/enrich/persist/insert_reupload_account.sql",
        artist_id,
        sc_user_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn compute_upload_kind(
    tx: &mut Transaction<'_, Postgres>,
    primary_artist_id: Option<Uuid>,
    uploader_sc_user_id: Option<&str>,
    source: ResolveSource,
) -> EnrichResult<&'static str> {
    let Some(artist_id) = primary_artist_id else {
        return Ok("unknown");
    };
    if let Some(sc_id) = uploader_sc_user_id
        && !sc_id.is_empty()
    {
        let account = sqlx::query_file!(
            "queries/enrich/persist/sc_account_role_verified.sql",
            artist_id,
            sc_id
        )
        .fetch_optional(&mut **tx)
        .await?
        .filter(|account| account.verified || account.source == "mb_resolve");
        if let Some(account) = account {
            return Ok(match account.role.as_str() {
                "main" => "original",
                "demo" => "demo",
                _ => "alt",
            });
        }
    }
    let external_credit = matches!(
        source,
        ResolveSource::Isrc | ResolveSource::Mb | ResolveSource::Genius | ResolveSource::ScVerified
    );
    Ok(if external_credit {
        "reupload"
    } else {
        "unknown"
    })
}
