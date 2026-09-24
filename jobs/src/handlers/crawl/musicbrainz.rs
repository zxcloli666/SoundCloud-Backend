use catalog_normalize::{normalize_title, title_forms};
use catalog_sources::{MbClient, MbRecordingBrief};
use sqlx::PgPool;
use tracing::debug;
use uuid::Uuid;

use super::entities;
use super::error::CrawlResult;

const PAGE_SIZE: u32 = 100;
const MAX_PAGES: usize = 10;

pub async fn discover_tracks(
    pool: &PgPool,
    mb: &MbClient,
    artist_id: Uuid,
    mb_artist_id: &str,
    starting_offset: u32,
) -> CrawlResult<u32> {
    let mut offset = starting_offset;
    for _ in 0..MAX_PAGES {
        let recordings = mb
            .browse_recordings_by_artist(mb_artist_id, offset, PAGE_SIZE)
            .await?;
        let count = recordings.len() as u32;
        for recording in recordings {
            if let Err(error) = persist_recording(pool, artist_id, mb_artist_id, recording).await {
                debug!(artist = %artist_id, %error, "musicbrainz recording persist failed");
            }
        }
        if count < PAGE_SIZE {
            return Ok(0);
        }
        offset += count;
    }
    Ok(offset)
}

async fn persist_recording(
    pool: &PgPool,
    crawled_artist_id: Uuid,
    crawled_mb_artist_id: &str,
    recording: MbRecordingBrief,
) -> CrawlResult {
    let primary_artist_id = match recording.primary_artist.as_ref() {
        Some(artist) if artist.mb_id == crawled_mb_artist_id => Some(crawled_artist_id),
        Some(artist) => entities::artist_id(pool, Some(&artist.mb_id), None, &artist.name).await?,
        None => None,
    };

    if let Some(isrc) = recording.isrc.as_deref()
        && entities::track_exists_with_isrc(pool, isrc).await?
    {
        return Ok(());
    }

    let normalized = normalize_title(&recording.title);
    if normalized.is_empty() {
        return Ok(());
    }
    let forms = title_forms(&recording.title);

    let album_id = match recording.release.as_ref() {
        Some(release) => entities::mb_album_id(pool, release, primary_artist_id).await?,
        None => None,
    };

    let wanted_id = sqlx::query_file_scalar!(
        "queries/crawl/upsert_wanted_from_mb.sql",
        recording.title.trim(),
        &normalized,
        primary_artist_id,
        recording.isrc.as_deref(),
        recording.length_ms,
        recording.first_release_year,
        &recording.mb_id,
        &forms.work_key,
        &forms.recording_key,
        catalog_normalize::NORMALIZER_VERSION
    )
    .fetch_optional(pool)
    .await?;
    let Some(wanted_id) = wanted_id else {
        return Ok(());
    };

    entities::store_wanted_aliases(pool, wanted_id, &forms.aliases).await?;
    if let Some(album_id) = album_id {
        entities::place_wanted_in_album(pool, wanted_id, album_id, 0).await?;
    }
    if let Some(primary_artist_id) = primary_artist_id {
        entities::credit_wanted_artist(pool, wanted_id, primary_artist_id, "primary", 0).await?;
    }
    for (position, featured) in recording.featured.iter().enumerate() {
        let Some(featured_id) =
            entities::artist_id(pool, Some(&featured.mb_id), None, &featured.name).await?
        else {
            continue;
        };
        entities::credit_wanted_artist(pool, wanted_id, featured_id, "featured", position as i16)
            .await?;
    }
    Ok(())
}
