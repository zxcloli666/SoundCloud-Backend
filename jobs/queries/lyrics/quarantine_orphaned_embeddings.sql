WITH oldest AS MATERIALIZED (
    SELECT lyrics.sc_track_id
    FROM lyrics_cache AS lyrics
    WHERE lyrics.embedded_at IS NULL
      AND lyrics.embedding_state IS NULL
      AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
      AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '48 hours'
    ORDER BY lyrics.created_at, lyrics.sc_track_id
    LIMIT $2
), candidates AS MATERIALIZED (
    SELECT lyrics.sc_track_id
    FROM oldest
    JOIN lyrics_cache AS lyrics ON lyrics.sc_track_id = oldest.sc_track_id
    WHERE lyrics.embedded_at IS NULL
      AND lyrics.embedding_state IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM tracks AS track
          WHERE track.sc_track_id = lyrics.sc_track_id
      )
    FOR UPDATE OF lyrics SKIP LOCKED
    LIMIT $1
), quarantined AS (
    UPDATE lyrics_cache AS lyrics
    SET embedding_state = 'quarantined'
    FROM candidates
    WHERE lyrics.sc_track_id = candidates.sc_track_id
    RETURNING lyrics.sc_track_id
)
SELECT count(*)::bigint AS "quarantined!"
FROM quarantined
