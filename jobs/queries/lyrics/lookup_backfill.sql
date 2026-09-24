WITH progress AS MATERIALIZED (
    SELECT cursor_id, completed
    FROM lyrics_lookup_backfill_state
    WHERE singleton
    FOR UPDATE
), page AS MATERIALIZED (
    SELECT track.id,
           track.sc_track_id,
           btrim(track.title) AS input_title,
           COALESCE(
               NULLIF(btrim(track.metadata_artist), ''),
               NULLIF(btrim(track.uploader_username), ''),
               ''
           ) AS input_artist,
           track.duration_ms,
           track.genius_song_id,
           NULLIF(btrim(track.genius_url), '') AS genius_url,
           track.release_date,
           track.sc_created_at,
           track.index_priority,
           track.created_at
    FROM tracks AS track
    JOIN progress ON NOT progress.completed
    WHERE progress.cursor_id IS NULL OR track.id > progress.cursor_id
    ORDER BY track.id
    LIMIT $1
), inserted AS (
    INSERT INTO lyrics_lookup_state (
        track_id,
        sc_track_id,
        priority,
        input_title,
        input_artist,
        input_duration_ms,
        input_genius_song_id,
        input_genius_url,
        input_release_date,
        input_sc_created_at,
        created_at
    )
    SELECT page.id,
           page.sc_track_id,
           page.index_priority,
           page.input_title,
           page.input_artist,
           page.duration_ms,
           page.genius_song_id,
           page.genius_url,
           page.release_date,
           page.sc_created_at,
           page.created_at
    FROM page
    WHERE NOT EXISTS (
        SELECT 1
        FROM lyrics_cache AS cache
        WHERE cache.sc_track_id = page.sc_track_id
          AND cache.source IN ('lrclib', 'musixmatch', 'genius', 'netease')
          AND (
              NULLIF(btrim(cache.plain_text), '') IS NOT NULL
              OR NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL
          )
    )
    ON CONFLICT (track_id) DO NOTHING
    RETURNING track_id
), updated AS (
    UPDATE lyrics_lookup_backfill_state AS state
    SET cursor_id = COALESCE(
            (SELECT id FROM page ORDER BY id DESC LIMIT 1),
            state.cursor_id
        ),
        rows_seen = state.rows_seen + (SELECT count(*) FROM page),
        rows_created = state.rows_created + (SELECT count(*) FROM inserted),
        completed = NOT EXISTS (SELECT 1 FROM page),
        updated_at = now()
    WHERE state.singleton
    RETURNING state.completed
)
SELECT completed AS "completed!"
FROM updated
