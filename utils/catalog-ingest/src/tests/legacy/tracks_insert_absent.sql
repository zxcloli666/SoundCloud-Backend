INSERT INTO tracks (
    sc_track_id, urn, title, title_normalized, description, genre, tags,
    duration_ms, artwork_url, permalink_url, waveform_url, language, isrc,
    metadata_artist, sharing, sc_created_at, sc_last_modified, release_year, release_date,
    uploader_sc_user_id, uploader_urn, uploader_username, uploader_avatar_url,
    play_count_sc, likes_count_sc, reposts_count_sc, comments_count_sc,
    needs_duration_resolve, index_priority, storage_priority, is_cover,
    sc_synced_at, sc_observation, sc_metadata, pipeline_held
)
SELECT row.sc_track_id, row.urn, row.title, row.title_normalized, row.description, row.genre,
       row.tags, row.duration_ms, row.artwork_url, row.permalink_url, row.waveform_url,
       row.language, row.isrc, row.metadata_artist, row.sharing, row.sc_created_at,
       row.sc_last_modified, row.release_year, row.release_date, row.uploader_sc_user_id,
       row.uploader_urn, row.uploader_username, row.uploader_avatar_url, row.play_count_sc,
       row.likes_count_sc, row.reposts_count_sc, row.comments_count_sc,
       row.needs_duration_resolve, $2, $2, row.is_cover, now(), $3, row.sc_metadata, true
FROM jsonb_to_recordset($1::jsonb) AS row(
    sc_track_id text, urn text, title text, title_normalized text, description text,
    genre text, tags text[], duration_ms integer, artwork_url text, permalink_url text,
    waveform_url text, language text, isrc text, metadata_artist text, sharing text,
    sc_created_at timestamptz, sc_last_modified timestamptz, release_year smallint,
    release_date date, uploader_sc_user_id text, uploader_urn text, uploader_username text,
    uploader_avatar_url text, play_count_sc bigint, likes_count_sc bigint,
    reposts_count_sc bigint, comments_count_sc bigint, needs_duration_resolve boolean,
    is_cover boolean, sc_metadata jsonb
)
ON CONFLICT DO NOTHING
RETURNING sc_track_id
