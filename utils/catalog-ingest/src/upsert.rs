use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Observation;
use crate::payload::ScTrackFields;

#[derive(Debug, Clone, Copy)]
pub enum TrackPriority {
    Like = 1,
    Playlist = 2,
    Played = 3,
    FreshDrop = 4,
    Discovery = 5,
}

impl TrackPriority {
    pub fn as_i16(self) -> i16 {
        self as i16
    }
}

pub struct IngestResult {
    pub id: Uuid,
    pub was_new: bool,
    pub metadata_applied: bool,
}

pub async fn upsert_from_sc(
    pool: &PgPool,
    fields: &ScTrackFields,
    new_index_priority: TrackPriority,
    new_storage_priority: TrackPriority,
    observation: Observation,
) -> Result<IngestResult, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    upsert_track_in(
        &mut connection,
        fields,
        new_index_priority,
        new_storage_priority,
        observation,
    )
    .await
}

pub async fn upsert_track_in(
    connection: &mut PgConnection,
    fields: &ScTrackFields,
    new_index_priority: TrackPriority,
    new_storage_priority: TrackPriority,
    observation: Observation,
) -> Result<IngestResult, sqlx::Error> {
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "INSERT INTO tracks (
            sc_track_id, urn, title, title_normalized, description, genre, tags,
            duration_ms, artwork_url, permalink_url, waveform_url, language, isrc,
            metadata_artist, sharing, sc_created_at, sc_last_modified, release_year, release_date,
            uploader_sc_user_id, uploader_urn, uploader_username, uploader_avatar_url,
            play_count_sc, likes_count_sc, reposts_count_sc, comments_count_sc,
            needs_duration_resolve, index_priority, storage_priority, is_cover, sc_synced_at, sc_observation, sc_metadata
         ) VALUES (
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,
            $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31, now(), $32, $33
         )
         ON CONFLICT (sc_track_id) DO UPDATE SET
            urn = EXCLUDED.urn,
            title = EXCLUDED.title,
            title_normalized = EXCLUDED.title_normalized,
            description = EXCLUDED.description,
            genre = EXCLUDED.genre,
            tags = EXCLUDED.tags,
            duration_ms = CASE
                WHEN EXCLUDED.duration_ms > 0 THEN EXCLUDED.duration_ms
                ELSE tracks.duration_ms
            END,
            storage_state = CASE
                WHEN tracks.storage_state = 'failed'
                     AND EXCLUDED.duration_ms > 0
                     AND EXCLUDED.duration_ms IS DISTINCT FROM tracks.duration_ms
                    THEN 'pending'
                ELSE tracks.storage_state
            END,
            storage_attempts = CASE
                WHEN tracks.storage_state = 'failed'
                     AND EXCLUDED.duration_ms > 0
                     AND EXCLUDED.duration_ms IS DISTINCT FROM tracks.duration_ms
                    THEN 0
                ELSE tracks.storage_attempts
            END,
            artwork_url = EXCLUDED.artwork_url,
            permalink_url = EXCLUDED.permalink_url,
            waveform_url = EXCLUDED.waveform_url,
            language = COALESCE(EXCLUDED.language, tracks.language),
            isrc = COALESCE(EXCLUDED.isrc, tracks.isrc),
            metadata_artist = COALESCE(EXCLUDED.metadata_artist, tracks.metadata_artist),
            sharing = EXCLUDED.sharing,
            sc_metadata = EXCLUDED.sc_metadata,
            sc_created_at = COALESCE(EXCLUDED.sc_created_at, tracks.sc_created_at),
            sc_last_modified = COALESCE(EXCLUDED.sc_last_modified, tracks.sc_last_modified),
            release_year = COALESCE(EXCLUDED.release_year, tracks.release_year),
            release_date = COALESCE(EXCLUDED.release_date, tracks.release_date),
            uploader_sc_user_id = COALESCE(EXCLUDED.uploader_sc_user_id, tracks.uploader_sc_user_id),
            uploader_urn = COALESCE(EXCLUDED.uploader_urn, tracks.uploader_urn),
            uploader_username = COALESCE(EXCLUDED.uploader_username, tracks.uploader_username),
            uploader_avatar_url = COALESCE(EXCLUDED.uploader_avatar_url, tracks.uploader_avatar_url),
            play_count_sc = COALESCE(EXCLUDED.play_count_sc, tracks.play_count_sc),
            likes_count_sc = COALESCE(EXCLUDED.likes_count_sc, tracks.likes_count_sc),
            reposts_count_sc = COALESCE(EXCLUDED.reposts_count_sc, tracks.reposts_count_sc),
            comments_count_sc = COALESCE(EXCLUDED.comments_count_sc, tracks.comments_count_sc),
            needs_duration_resolve = EXCLUDED.needs_duration_resolve,
            duration_resolve_attempts = CASE
                WHEN tracks.needs_duration_resolve IS DISTINCT FROM EXCLUDED.needs_duration_resolve
                    THEN 0
                ELSE tracks.duration_resolve_attempts
            END,
            duration_resolve_retry_at = CASE
                WHEN tracks.needs_duration_resolve IS DISTINCT FROM EXCLUDED.needs_duration_resolve
                    THEN NULL
                ELSE tracks.duration_resolve_retry_at
            END,
            index_priority = LEAST(tracks.index_priority, EXCLUDED.index_priority),
            storage_priority = LEAST(tracks.storage_priority, EXCLUDED.storage_priority),
            is_cover = tracks.is_cover OR EXCLUDED.is_cover,
            sc_synced_at = now(),
            sc_observation = GREATEST(tracks.sc_observation, EXCLUDED.sc_observation),
            sc_desired = '{}',
            sc_write_confirmed = false,
            updated_at = now()
         WHERE tracks.deleted_at IS NULL AND catalog_observation_is_current(
            tracks.sc_observation, tracks.sc_mutation_observation, EXCLUDED.sc_observation,
            tracks.sc_last_modified, COALESCE(EXCLUDED.sc_last_modified, tracks.sc_last_modified),
            tracks.sc_desired, tracks.sc_write_confirmed, to_jsonb(EXCLUDED)
         )
         RETURNING id, (xmax = 0) AS was_new",
    )
    .bind(&fields.sc_track_id)
    .bind(&fields.urn)
    .bind(&fields.title)
    .bind(&fields.title_normalized)
    .bind(&fields.description)
    .bind(&fields.genre)
    .bind(&fields.tags)
    .bind(fields.duration_ms)
    .bind(&fields.artwork_url)
    .bind(&fields.permalink_url)
    .bind(&fields.waveform_url)
    .bind(&fields.language)
    .bind(&fields.isrc)
    .bind(&fields.metadata_artist)
    .bind(&fields.sharing)
    .bind(fields.sc_created_at)
    .bind(fields.sc_last_modified)
    .bind(fields.release_year)
    .bind(fields.release_date)
    .bind(&fields.uploader_sc_user_id)
    .bind(&fields.uploader_urn)
    .bind(&fields.uploader_username)
    .bind(&fields.uploader_avatar_url)
    .bind(fields.play_count_sc)
    .bind(fields.likes_count_sc)
    .bind(fields.reposts_count_sc)
    .bind(fields.comments_count_sc)
    .bind(fields.needs_duration_resolve)
    .bind(new_index_priority.as_i16())
    .bind(new_storage_priority.as_i16())
    .bind(fields.is_cover)
    .bind(observation.sequence())
    .bind(&fields.sc_metadata)
    .fetch_optional(&mut *connection)
    .await?;

    match row {
        Some(r) => Ok(IngestResult {
            id: r.0,
            was_new: r.1,
            metadata_applied: true,
        }),
        None => {
            sqlx::query_file!(
                "queries/tracks/bump_priority.sql",
                &fields.sc_track_id,
                new_index_priority.as_i16(),
                new_storage_priority.as_i16()
            )
            .execute(&mut *connection)
            .await?;
            let existing: (Uuid,) = sqlx::query_as("SELECT id FROM tracks WHERE sc_track_id = $1")
                .bind(&fields.sc_track_id)
                .fetch_one(&mut *connection)
                .await?;
            Ok(IngestResult {
                id: existing.0,
                was_new: false,
                metadata_applied: false,
            })
        }
    }
}
