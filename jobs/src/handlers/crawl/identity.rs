use catalog_normalize::{is_junk_artist_name, name_similarity, normalize_name};
use catalog_sources::GeniusService;
use sqlx::PgPool;
use tracing::{debug, info};
use uuid::Uuid;

use super::error::CrawlResult;

const SEARCH_LIMIT: usize = 10;
const NAME_THRESHOLD: f32 = 0.92;
const MIN_NAME_CHARS: usize = 3;

pub async fn resolve_genius_id(
    pool: &PgPool,
    genius: &GeniusService,
    artist_id: Uuid,
    name: &str,
    retry_after_days: f64,
) -> CrawlResult<bool> {
    let Some(genius_artist_id) = look_up(genius, name).await? else {
        return defer(pool, artist_id, retry_after_days).await;
    };

    attach(pool, artist_id, genius_artist_id, name, retry_after_days).await
}

pub(super) async fn attach(
    pool: &PgPool,
    artist_id: Uuid,
    genius_artist_id: i64,
    name: &str,
    retry_after_days: f64,
) -> CrawlResult<bool> {
    let genius_artist_id = genius_artist_id.to_string();
    let attached = sqlx::query_file!(
        "queries/crawl/attach_genius_artist_id.sql",
        artist_id,
        &genius_artist_id
    )
    .execute(pool)
    .await;
    match attached {
        Ok(done) if done.rows_affected() > 0 => {
            info!(%artist_id, genius_artist_id, name, "verified artist matched on genius");
            Ok(true)
        }
        Ok(_) => {
            let held_by_another: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM artists WHERE genius_artist_id = $1 AND id <> $2)",
            )
            .bind(&genius_artist_id)
            .bind(artist_id)
            .fetch_one(pool)
            .await?;
            if !held_by_another {
                return Ok(true);
            }
            debug!(%artist_id, genius_artist_id, "genius id is held by another artist");
            defer(pool, artist_id, retry_after_days).await
        }
        Err(error) if is_unique_violation(&error) => {
            debug!(%artist_id, genius_artist_id, "genius id was taken by another artist");
            defer(pool, artist_id, retry_after_days).await
        }
        Err(error) => Err(error.into()),
    }
}

async fn defer(pool: &PgPool, artist_id: Uuid, retry_after_days: f64) -> CrawlResult<bool> {
    sqlx::query_file!(
        "queries/crawl/defer_identity_lookup.sql",
        artist_id,
        retry_after_days
    )
    .execute(pool)
    .await?;
    Ok(false)
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
}

async fn look_up(genius: &GeniusService, name: &str) -> CrawlResult<Option<i64>> {
    let normalized = normalize_name(name);
    if normalized.chars().count() < MIN_NAME_CHARS || is_junk_artist_name(name) {
        debug!(name, "artist name is too weak to claim a genius identity");
        return Ok(None);
    }

    let candidates = genius.search_artist(name, SEARCH_LIMIT).await?;
    let mut best: Option<(f32, i64)> = None;
    for candidate in candidates {
        let Some(candidate_id) = candidate.genius_artist_id else {
            continue;
        };
        let similarity = name_similarity(name, &candidate.name);
        if similarity < NAME_THRESHOLD {
            continue;
        }
        if best.is_none_or(|(best_similarity, _)| similarity > best_similarity) {
            best = Some((similarity, candidate_id));
        }
    }
    Ok(best.map(|(_, candidate_id)| candidate_id))
}
