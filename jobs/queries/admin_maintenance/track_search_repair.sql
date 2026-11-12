WITH locked AS MATERIALIZED (
    SELECT track.id,
           track.sc_track_id,
           track.uploader_sc_user_id,
           track.title,
           track.title_normalized,
           track.metadata_artist,
           track.uploader_username,
           track.duration_ms,
           track.play_count_sc,
           track.sharing = 'public' AND track.deleted_at IS NULL AND track.superseded_by IS NULL AS searchable
    FROM tracks AS track
    WHERE track.id = ANY ($1)
    ORDER BY track.id
    FOR SHARE
), current AS MATERIALIZED (
    SELECT locked.id,
           locked.sc_track_id,
           locked.uploader_sc_user_id,
           locked.title,
           locked.title_normalized,
           locked.metadata_artist,
           locked.uploader_username,
           locked.duration_ms,
           locked.play_count_sc,
           track_search_variant(locked.title_normalized) AS variant
    FROM locked
    WHERE locked.searchable
), removed AS (
    DELETE FROM track_search AS search
    WHERE search.id = ANY ($1)
      AND NOT EXISTS (SELECT 1 FROM current WHERE current.id = search.id)
    RETURNING search.id
), written AS (
    INSERT INTO track_search AS search (
        id, sc_track_id, uploader_sc_user_id, title, title_normalized,
        metadata_artist, uploader_username, duration_ms, play_count_sc, variant
    )
    SELECT current.id,
           current.sc_track_id,
           current.uploader_sc_user_id,
           current.title,
           current.title_normalized,
           current.metadata_artist,
           current.uploader_username,
           current.duration_ms,
           current.play_count_sc,
           current.variant
    FROM current
    ORDER BY current.id
    ON CONFLICT (id) DO UPDATE
    SET sc_track_id = EXCLUDED.sc_track_id,
        uploader_sc_user_id = EXCLUDED.uploader_sc_user_id,
        title = EXCLUDED.title,
        title_normalized = EXCLUDED.title_normalized,
        metadata_artist = EXCLUDED.metadata_artist,
        uploader_username = EXCLUDED.uploader_username,
        duration_ms = EXCLUDED.duration_ms,
        play_count_sc = EXCLUDED.play_count_sc,
        variant = EXCLUDED.variant
    WHERE (search.sc_track_id, search.uploader_sc_user_id, search.title, search.title_normalized,
           search.metadata_artist, search.uploader_username, search.duration_ms, search.play_count_sc,
           search.variant)
          IS DISTINCT FROM
          (EXCLUDED.sc_track_id, EXCLUDED.uploader_sc_user_id, EXCLUDED.title, EXCLUDED.title_normalized,
           EXCLUDED.metadata_artist, EXCLUDED.uploader_username, EXCLUDED.duration_ms, EXCLUDED.play_count_sc,
           EXCLUDED.variant)
    RETURNING search.id
)
SELECT (SELECT count(*) FROM removed) AS "removed!",
       (SELECT count(*) FROM written) AS "written!"
