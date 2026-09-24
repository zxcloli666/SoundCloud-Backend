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
    wake_message_id,
    wake_generation,
    created_at
)
SELECT track.id,
       track.sc_track_id,
       0,
       btrim(track.title),
       COALESCE(
           NULLIF(btrim(track.metadata_artist), ''),
           NULLIF(btrim(track.uploader_username), ''),
           ''
       ),
       track.duration_ms,
       track.genius_song_id,
       NULLIF(btrim(track.genius_url), ''),
       track.release_date,
       track.sc_created_at,
       gen_random_uuid(),
       1,
       track.created_at
FROM tracks AS track
WHERE track.sc_track_id = $1
  AND NOT EXISTS (
      SELECT 1
      FROM lyrics_cache AS cache
      WHERE cache.sc_track_id = track.sc_track_id
        AND (
            NULLIF(btrim(cache.plain_text), '') IS NOT NULL
            OR NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL
        )
  )
ON CONFLICT (track_id) DO NOTHING
