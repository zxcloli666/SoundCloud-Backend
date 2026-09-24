use catalog_normalize::{clean_artist_name, normalize_name, normalize_title};
use catalog_sources::{GeniusAlbumRef, MbReleaseBrief};
use sqlx::PgPool;
use uuid::Uuid;

use super::error::CrawlResult;

pub async fn artist_id(
    pool: &PgPool,
    mb_artist_id: Option<&str>,
    genius_artist_id: Option<&str>,
    name: &str,
) -> CrawlResult<Option<Uuid>> {
    if let Some(mb_artist_id) = mb_artist_id
        && let Some(id) =
            sqlx::query_file_scalar!("queries/crawl/artist_id_by_mb_artist_id.sql", mb_artist_id)
                .fetch_optional(pool)
                .await?
    {
        return Ok(Some(id));
    }
    if let Some(genius_artist_id) = genius_artist_id
        && let Some(id) = sqlx::query_file_scalar!(
            "queries/crawl/artist_id_by_genius_artist_id.sql",
            genius_artist_id
        )
        .fetch_optional(pool)
        .await?
    {
        return Ok(Some(id));
    }

    let cleaned = clean_artist_name(name);
    let normalized = normalize_name(&cleaned);
    if cleaned.is_empty() || normalized.is_empty() {
        return Ok(None);
    }

    if let Some(id) = sqlx::query_file_scalar!(
        "queries/crawl/artist_id_by_normalized_name.sql",
        &normalized
    )
    .fetch_optional(pool)
    .await?
    {
        if mb_artist_id.is_some() || genius_artist_id.is_some() {
            sqlx::query_file!(
                "queries/crawl/update_artist_external_ids.sql",
                id,
                mb_artist_id,
                genius_artist_id
            )
            .execute(pool)
            .await?;
        }
        return Ok(Some(id));
    }

    let inserted = sqlx::query_file_scalar!(
        "queries/crawl/insert_artist.sql",
        &cleaned,
        &normalized,
        mb_artist_id,
        genius_artist_id
    )
    .fetch_optional(pool)
    .await?;
    if inserted.is_some() {
        return Ok(inserted);
    }
    Ok(sqlx::query_file_scalar!(
        "queries/crawl/artist_id_by_normalized_name.sql",
        &normalized
    )
    .fetch_optional(pool)
    .await?)
}

pub async fn mb_album_id(
    pool: &PgPool,
    release: &MbReleaseBrief,
    primary_artist_id: Option<Uuid>,
) -> CrawlResult<Option<Uuid>> {
    if let Some(id) = sqlx::query_file_scalar!(
        "queries/crawl/album_id_by_mb_release_id.sql",
        &release.mb_id
    )
    .fetch_optional(pool)
    .await?
    {
        return Ok(Some(id));
    }

    let normalized = normalize_title(&release.title);
    if normalized.is_empty() {
        return Ok(None);
    }
    let kind = match release.release_type.as_deref() {
        Some("EP") => "ep",
        Some("Single") => "single",
        Some("Compilation") => "compilation",
        _ => "album",
    };
    let inserted = sqlx::query_file_scalar!(
        "queries/crawl/insert_mb_album.sql",
        release.title.trim(),
        &normalized,
        primary_artist_id,
        kind,
        release.year,
        &release.mb_id
    )
    .fetch_optional(pool)
    .await?;
    if let Some(id) = inserted {
        credit_album_artist(pool, id, primary_artist_id).await?;
        return Ok(Some(id));
    }
    Ok(sqlx::query_file_scalar!(
        "queries/crawl/album_id_by_mb_release_id.sql",
        &release.mb_id
    )
    .fetch_optional(pool)
    .await?)
}

pub async fn genius_album_id(
    pool: &PgPool,
    album: GeniusAlbumRef,
    primary_artist_id: Option<Uuid>,
    year_hint: Option<i16>,
) -> CrawlResult<Option<Uuid>> {
    let genius_album_id = album.genius_album_id.to_string();
    if let Some(id) = sqlx::query_file_scalar!(
        "queries/crawl/album_id_by_genius_album_id.sql",
        &genius_album_id
    )
    .fetch_optional(pool)
    .await?
    {
        sqlx::query_file!(
            "queries/crawl/update_album_cover_year.sql",
            id,
            album.cover_url.as_deref(),
            album.year.or(year_hint)
        )
        .execute(pool)
        .await?;
        credit_album_artist(pool, id, primary_artist_id).await?;
        return Ok(Some(id));
    }

    let normalized = normalize_title(&album.name);
    if normalized.is_empty() {
        return Ok(None);
    }
    let inserted = sqlx::query_file_scalar!(
        "queries/crawl/insert_genius_album.sql",
        album.name.trim(),
        &normalized,
        primary_artist_id,
        album.year.or(year_hint),
        &genius_album_id,
        album.cover_url.as_deref()
    )
    .fetch_optional(pool)
    .await?;
    if let Some(id) = inserted {
        credit_album_artist(pool, id, primary_artist_id).await?;
        return Ok(Some(id));
    }
    Ok(sqlx::query_file_scalar!(
        "queries/crawl/album_id_by_genius_album_id.sql",
        &genius_album_id
    )
    .fetch_optional(pool)
    .await?)
}

async fn credit_album_artist(
    pool: &PgPool,
    album_id: Uuid,
    primary_artist_id: Option<Uuid>,
) -> CrawlResult {
    let Some(artist_id) = primary_artist_id else {
        return Ok(());
    };
    sqlx::query_file!(
        "queries/crawl/insert_album_artist_primary.sql",
        album_id,
        artist_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn credit_wanted_artist(
    pool: &PgPool,
    wanted_id: Uuid,
    artist_id: Uuid,
    role: &str,
    position: i16,
) -> CrawlResult {
    sqlx::query_file!(
        "queries/crawl/insert_wanted_track_artist.sql",
        wanted_id,
        artist_id,
        role,
        position
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn place_wanted_in_album(
    pool: &PgPool,
    wanted_id: Uuid,
    album_id: Uuid,
    position: i16,
) -> CrawlResult {
    sqlx::query_file!(
        "queries/crawl/insert_wanted_track_album.sql",
        wanted_id,
        album_id,
        position
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn place_track_in_album(
    pool: &PgPool,
    track_id: Uuid,
    album_id: Uuid,
    position: Option<i16>,
) -> CrawlResult {
    match position {
        Some(position) => {
            sqlx::query_file!(
                "queries/crawl/link_track_album_with_position.sql",
                track_id,
                album_id,
                position
            )
            .execute(pool)
            .await?;
            sqlx::query_file!(
                "queries/crawl/insert_album_track_with_position.sql",
                album_id,
                track_id,
                position
            )
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query_file!("queries/crawl/link_track_album.sql", track_id, album_id)
                .execute(pool)
                .await?;
            sqlx::query_file!("queries/crawl/insert_album_track.sql", album_id, track_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

pub async fn track_exists_with_isrc(pool: &PgPool, isrc: &str) -> CrawlResult<bool> {
    let existing = sqlx::query_file_scalar!("queries/crawl/track_id_by_isrc.sql", isrc)
        .fetch_optional(pool)
        .await?;
    Ok(existing.is_some())
}

pub async fn store_wanted_aliases(
    pool: &PgPool,
    wanted_id: Uuid,
    aliases: &[String],
) -> CrawlResult {
    sqlx::query_file!(
        "queries/crawl/replace_wanted_aliases.sql",
        wanted_id,
        aliases
    )
    .execute(pool)
    .await?;
    Ok(())
}
