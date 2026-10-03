use std::collections::HashSet;
use std::time::Instant;

use sqlx::{PgPool, Postgres, Transaction};
use tracing::debug;
use uuid::Uuid;

use super::failure;
use super::rank::Candidate;
use super::repository::{self, begin, configure_search};
use super::terms::{QueryTerms, Span};
use crate::error::AppResult;
use crate::modules::tracks::TrackRow;

const SCOPED_TIMEOUT_MS: i32 = 800;
const TITLED_TIMEOUT_MS: i32 = 1500;
const SINGLE_TOKEN_POOL: i64 = 200;
const TRANSLIT_BELOW: usize = 10;

struct CandidateRow {
    id: Uuid,
    title: String,
    uploader_username: Option<String>,
    metadata_artist: Option<String>,
    play_count_sc: Option<i64>,
    access: Option<String>,
    linked: bool,
}

impl CandidateRow {
    fn into_candidate(self, spanned: bool) -> Candidate {
        Candidate {
            key: self.id.to_string(),
            title: self.title,
            uploader: self.uploader_username,
            metadata_artist: self.metadata_artist,
            plays: self.play_count_sc.unwrap_or(0),
            preview: self.access.as_deref() == Some("preview"),
            spanned,
            linked: self.linked,
            sc_rank: None,
        }
    }
}

pub struct Pool {
    pub candidates: Vec<Candidate>,
    pub span: Option<Span>,
    pub partial: bool,
}

pub async fn gather(pg: &PgPool, raw: &str, terms: &QueryTerms) -> AppResult<Pool> {
    let (scoped, titled) = tokio::join!(scoped(pg, terms), titled_or_latin(pg, raw, terms));
    match (scoped, titled) {
        (Ok((span, spanned)), Ok(titled)) => Ok(Pool {
            candidates: merged(spanned, titled),
            span,
            partial: false,
        }),
        (Ok((span, spanned)), Err(error)) if !spanned.is_empty() => {
            debug!(%error, "title candidates failed, ranking the artist candidates alone");
            Ok(Pool {
                candidates: spanned,
                span,
                partial: true,
            })
        }
        (Err(error), Ok(titled)) if !titled.is_empty() => {
            debug!(%error, "artist candidates failed, ranking the title candidates alone");
            Ok(Pool {
                candidates: titled,
                span: None,
                partial: true,
            })
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

pub async fn span_artists(pg: &PgPool, names: &[String]) -> AppResult<Vec<Uuid>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut tx = begin(pg).await?;
    configure_search(&mut tx, SCOPED_TIMEOUT_MS).await?;
    let rows = sqlx::query_file!("queries/search/ranked/artist_spans.sql", names)
        .fetch_all(&mut *tx)
        .await
        .map_err(failure::from_db)?;
    tx.commit().await.map_err(failure::from_db)?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

async fn scoped(pg: &PgPool, terms: &QueryTerms) -> AppResult<(Option<Span>, Vec<Candidate>)> {
    let names = terms.span_names();
    if names.is_empty() {
        return Ok((None, Vec::new()));
    }
    let started = Instant::now();
    let mut tx = begin(pg).await?;
    configure_search(&mut tx, SCOPED_TIMEOUT_MS).await?;
    let artists = sqlx::query_file!("queries/search/ranked/artist_spans.sql", &names)
        .fetch_all(&mut *tx)
        .await
        .map_err(failure::from_db)?;
    let uploaders = sqlx::query_file!("queries/search/ranked/uploader_spans.sql", &names)
        .fetch_all(&mut *tx)
        .await
        .map_err(failure::from_db)?;
    crate::metrics::record_search_phase("a", started.elapsed());

    let found: HashSet<&str> = artists
        .iter()
        .map(|row| row.normalized_name.as_str())
        .chain(uploaders.iter().map(|row| row.username_normalized.as_str()))
        .collect();
    let Some(span) = best_span(terms, &found) else {
        tx.commit().await.map_err(failure::from_db)?;
        return Ok((None, Vec::new()));
    };
    let names_at_span: HashSet<&str> = terms
        .spans
        .iter()
        .filter(|variant| variant.start == span.start && variant.len == span.len)
        .map(|variant| variant.text.as_str())
        .collect();
    let artist_ids: Vec<Uuid> = artists
        .iter()
        .filter(|row| names_at_span.contains(row.normalized_name.as_str()))
        .map(|row| row.id)
        .collect();
    let uploader_ids: Vec<String> = uploaders
        .iter()
        .filter(|row| names_at_span.contains(row.username_normalized.as_str()))
        .map(|row| row.sc_user_id.clone())
        .collect();

    let started = Instant::now();
    let rows = sqlx::query_file_as!(
        CandidateRow,
        "queries/search/ranked/candidates_scoped.sql",
        &artist_ids,
        &uploader_ids,
        &terms.rest_slots(&span)
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(failure::from_db)?;
    tx.commit().await.map_err(failure::from_db)?;
    crate::metrics::record_search_phase("b", started.elapsed());
    let candidates = rows
        .into_iter()
        .map(|row| row.into_candidate(true))
        .collect();
    Ok((Some(span), candidates))
}

fn best_span(terms: &QueryTerms, found: &HashSet<&str>) -> Option<Span> {
    terms
        .spans
        .iter()
        .filter(|span| found.contains(span.text.as_str()))
        .max_by_key(|span| (span.len, span.start == 0))
        .cloned()
}

async fn titled_or_latin(pg: &PgPool, raw: &str, terms: &QueryTerms) -> AppResult<Vec<Candidate>> {
    let found = titled(pg, raw, terms).await?;
    let Some(latin) = terms.latin().filter(|_| found.len() < TRANSLIT_BELOW) else {
        return Ok(found);
    };
    let extra = titled(pg, &latin.norm, &latin).await?;
    Ok(merged(found, extra))
}

async fn titled(pg: &PgPool, raw: &str, terms: &QueryTerms) -> AppResult<Vec<Candidate>> {
    let started = Instant::now();
    let mut tx = begin(pg).await?;
    configure_search(&mut tx, TITLED_TIMEOUT_MS).await?;
    let candidates = match terms.slots.as_slice() {
        [first, rest @ ..] if !terms.is_single() => title_tokens(&mut tx, first, rest).await?,
        _ => popular_pool(&mut tx, raw, terms.prefix_only).await?,
    };
    tx.commit().await.map_err(failure::from_db)?;
    crate::metrics::record_search_phase("c", started.elapsed());
    Ok(candidates)
}

async fn title_tokens(
    tx: &mut Transaction<'_, Postgres>,
    first: &str,
    rest: &[String],
) -> AppResult<Vec<Candidate>> {
    let slot = |at: usize| rest.get(at).map(String::as_str);
    let rows = sqlx::query_file_as!(
        CandidateRow,
        "queries/search/ranked/candidates_title_tokens.sql",
        first,
        slot(0),
        slot(1),
        slot(2)
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(failure::from_db)?;
    Ok(rows
        .into_iter()
        .map(|row| row.into_candidate(false))
        .collect())
}

async fn popular_pool(
    tx: &mut Transaction<'_, Postgres>,
    raw: &str,
    prefix_only: bool,
) -> AppResult<Vec<Candidate>> {
    let rows = sqlx::query_file_as!(
        TrackRow,
        "queries/search/repository/search_tracks_global.sql",
        repository::like_needle(raw),
        SINGLE_TOKEN_POOL,
        0_i64,
        repository::like_needle_normalized(raw),
        None::<&[String]>,
        None::<String>,
        None::<&[String]>,
        prefix_only
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(failure::from_db)?;
    Ok(rows.into_iter().map(track_candidate).collect())
}

fn track_candidate(row: TrackRow) -> Candidate {
    let preview = row
        .sc_metadata
        .get("access")
        .and_then(|value| value.as_str())
        == Some("preview");
    Candidate {
        key: row.id.to_string(),
        title: row.title,
        uploader: row.uploader_username,
        metadata_artist: row.metadata_artist,
        plays: row.play_count_sc.unwrap_or(0),
        preview,
        ..Candidate::default()
    }
}

fn merged(first: Vec<Candidate>, second: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen: HashSet<String> = HashSet::new();
    first
        .into_iter()
        .chain(second)
        .filter(|candidate| seen.insert(candidate.key.clone()))
        .collect()
}
