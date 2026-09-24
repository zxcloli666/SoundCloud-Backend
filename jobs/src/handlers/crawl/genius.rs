use catalog_normalize::{NORMALIZER_VERSION, normalize_title, title_forms};
use catalog_sources::{GeniusAlbumTrack, GeniusService, GeniusSongMeta};
use sqlx::PgPool;
use tracing::debug;
use uuid::Uuid;

use super::entities;
use super::error::CrawlResult;

const SONG_PAGE_SIZE: u32 = 50;
const MAX_SONG_PAGES: usize = 10;
const ALBUM_PAGE_SIZE: u32 = 20;
const MAX_ALBUM_PAGES: usize = 10;
const ALBUM_TRACK_PAGE_SIZE: u32 = 50;
const MAX_ALBUM_TRACK_PAGES: usize = 6;

pub async fn discover_songs(
    pool: &PgPool,
    genius: &GeniusService,
    artist_id: Uuid,
    genius_artist_id: i64,
    starting_offset: u32,
) -> CrawlResult<u32> {
    let (songs, next_offset) = genius
        .list_artist_songs_window(
            genius_artist_id,
            starting_offset,
            SONG_PAGE_SIZE,
            MAX_SONG_PAGES,
        )
        .await?;
    for song in songs {
        if let Err(error) = persist_song(pool, genius, artist_id, genius_artist_id, song).await {
            debug!(artist = %artist_id, %error, "genius song persist failed");
        }
    }
    Ok(next_offset)
}

pub async fn discover_albums(
    pool: &PgPool,
    genius: &GeniusService,
    artist_id: Uuid,
    genius_artist_id: i64,
) -> CrawlResult {
    let albums = genius
        .list_artist_albums_window(genius_artist_id, ALBUM_PAGE_SIZE, MAX_ALBUM_PAGES)
        .await?;
    for album in albums {
        let year_hint = album.year;
        let genius_album_id = album.genius_album_id;
        let album_id =
            match entities::genius_album_id(pool, album, Some(artist_id), year_hint).await {
                Ok(Some(album_id)) => album_id,
                Ok(None) => continue,
                Err(error) => {
                    debug!(artist = %artist_id, %error, "genius album upsert failed");
                    continue;
                }
            };
        if let Err(error) =
            ingest_album_tracks(pool, genius, artist_id, album_id, genius_album_id).await
        {
            debug!(%album_id, %error, "genius album tracks ingest failed");
        }
    }
    Ok(())
}

pub async fn ingest_album_tracks(
    pool: &PgPool,
    genius: &GeniusService,
    primary_artist_id: Uuid,
    album_id: Uuid,
    genius_album_id: i64,
) -> CrawlResult {
    let tracks = genius
        .list_album_tracks_window(
            genius_album_id,
            ALBUM_TRACK_PAGE_SIZE,
            MAX_ALBUM_TRACK_PAGES,
        )
        .await?;
    for track in tracks {
        if let Err(error) = persist_album_track(pool, primary_artist_id, album_id, track).await {
            debug!(%album_id, %error, "genius album track persist failed");
        }
    }
    Ok(())
}

async fn persist_song(
    pool: &PgPool,
    genius: &GeniusService,
    crawled_artist_id: Uuid,
    crawled_genius_artist_id: i64,
    song: GeniusSongMeta,
) -> CrawlResult {
    let Some(primary) = song.primary_artist else {
        return Ok(());
    };
    let Some(genius_song_id) = song.genius_song_id else {
        return Ok(());
    };
    let primary_artist_id = match primary.genius_artist_id {
        Some(id) if id == crawled_genius_artist_id => Some(crawled_artist_id),
        _ => {
            entities::artist_id(
                pool,
                None,
                primary.genius_artist_id.map(|id| id.to_string()).as_deref(),
                &primary.name,
            )
            .await?
        }
    };

    let normalized = normalize_title(&song.title);
    if normalized.is_empty() {
        return Ok(());
    }

    if let Some(primary_artist_id) = primary_artist_id
        && let Some(local) =
            catalog_match::best_indexed_for_artist_title(pool, primary_artist_id, &song.title)
                .await?
    {
        catalog_match::attach_genius_song(pool, local.track_id, genius_song_id).await?;
        if let Some(album) = genius
            .lookup_song(genius_song_id)
            .await
            .and_then(|s| s.album)
        {
            let year_hint = album.year;
            if let Some(album_id) =
                entities::genius_album_id(pool, album, Some(primary_artist_id), year_hint).await?
            {
                entities::place_track_in_album(pool, local.track_id, album_id, None).await?;
            }
        }
        return Ok(());
    }

    let forms = title_forms(&song.title);
    let external_id = genius_song_id.to_string();
    let wanted_id = sqlx::query_file_scalar!(
        "queries/crawl/upsert_wanted_from_genius.sql",
        song.title.trim(),
        &normalized,
        primary_artist_id,
        &external_id,
        &forms.work_key,
        &forms.recording_key,
        NORMALIZER_VERSION
    )
    .fetch_optional(pool)
    .await?;
    let Some(wanted_id) = wanted_id else {
        return Ok(());
    };

    entities::store_wanted_aliases(pool, wanted_id, &forms.aliases).await?;
    if let Some(primary_artist_id) = primary_artist_id {
        entities::credit_wanted_artist(pool, wanted_id, primary_artist_id, "primary", 0).await?;
    }
    credit_featured(pool, wanted_id, &song.featured).await?;

    if let Some(details) = genius.lookup_song(genius_song_id).await
        && let Some(album) = details.album
    {
        let year_hint = details.year;
        if let Some(album_id) =
            entities::genius_album_id(pool, album, primary_artist_id, year_hint).await?
        {
            entities::place_wanted_in_album(pool, wanted_id, album_id, 0).await?;
        }
    }
    Ok(())
}

async fn persist_album_track(
    pool: &PgPool,
    album_primary_artist_id: Uuid,
    album_id: Uuid,
    track: GeniusAlbumTrack,
) -> CrawlResult {
    let normalized = normalize_title(&track.title);
    if normalized.is_empty() {
        return Ok(());
    }
    let primary_artist_id = match track.primary_artist.as_ref() {
        Some(primary) => entities::artist_id(
            pool,
            None,
            primary.genius_artist_id.map(|id| id.to_string()).as_deref(),
            &primary.name,
        )
        .await?
        .or(Some(album_primary_artist_id)),
        None => Some(album_primary_artist_id),
    };
    let position = track
        .position
        .and_then(|position| i16::try_from(position).ok())
        .unwrap_or(0);

    if let Some(primary_artist_id) = primary_artist_id
        && let Some(local) =
            catalog_match::best_indexed_for_artist_title(pool, primary_artist_id, &track.title)
                .await?
    {
        catalog_match::attach_genius_song(pool, local.track_id, track.genius_song_id).await?;
        entities::place_track_in_album(pool, local.track_id, album_id, Some(position)).await?;
        return Ok(());
    }

    let forms = title_forms(&track.title);
    let external_id = track.genius_song_id.to_string();
    let wanted_id = sqlx::query_file_scalar!(
        "queries/crawl/upsert_wanted_from_genius.sql",
        track.title.trim(),
        &normalized,
        primary_artist_id,
        &external_id,
        &forms.work_key,
        &forms.recording_key,
        NORMALIZER_VERSION
    )
    .fetch_optional(pool)
    .await?;
    let Some(wanted_id) = wanted_id else {
        return Ok(());
    };

    entities::store_wanted_aliases(pool, wanted_id, &forms.aliases).await?;
    if let Some(primary_artist_id) = primary_artist_id {
        entities::credit_wanted_artist(pool, wanted_id, primary_artist_id, "primary", 0).await?;
    }
    credit_featured(pool, wanted_id, &track.featured).await?;
    entities::place_wanted_in_album(pool, wanted_id, album_id, position).await?;
    Ok(())
}

async fn credit_featured(
    pool: &PgPool,
    wanted_id: Uuid,
    featured: &[catalog_sources::GeniusArtistRef],
) -> CrawlResult {
    for (position, artist) in featured.iter().enumerate() {
        let Some(artist_id) = entities::artist_id(
            pool,
            None,
            artist.genius_artist_id.map(|id| id.to_string()).as_deref(),
            &artist.name,
        )
        .await?
        else {
            continue;
        };
        entities::credit_wanted_artist(pool, wanted_id, artist_id, "featured", position as i16)
            .await?;
    }
    Ok(())
}
