WITH idle AS MATERIALIZED (
    SELECT NOT EXISTS (
        SELECT 1
        FROM lyrics_lookup_queue AS queue
        WHERE queue.next_run_at <= now()
          AND queue.claim_job_id IS NULL
    ) AS yes
), stale AS MATERIALIZED (
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
           track.created_at
    FROM lyrics_cache AS cache
    JOIN tracks AS track ON track.sc_track_id = cache.sc_track_id
    WHERE (SELECT yes FROM idle)
      AND cache.source = 'lrclib'
      AND cache.synced_lrc IS NULL
      AND cache.updated_at < now() - $2::integer * interval '1 day'
      AND NOT track.pipeline_held
      AND track.deleted_at IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM lyrics_lookup_state AS state
          WHERE state.track_id = track.id
      )
    ORDER BY cache.updated_at, cache.sc_track_id
    LIMIT $1
)
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
SELECT stale.id,
       stale.sc_track_id,
       5,
       stale.input_title,
       stale.input_artist,
       stale.duration_ms,
       stale.genius_song_id,
       stale.genius_url,
       stale.release_date,
       stale.sc_created_at,
       stale.created_at
FROM stale
ON CONFLICT (track_id) DO NOTHING
