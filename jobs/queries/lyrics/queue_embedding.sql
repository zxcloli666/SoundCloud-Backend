WITH candidate AS MATERIALIZED (
    SELECT lyrics.sc_track_id
    FROM lyrics_cache AS lyrics
    WHERE lyrics.sc_track_id = $1
      AND lyrics.embedded_at IS NULL
      AND lyrics.embedding_state IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM lyrics_embedding_wire_state AS wire
          WHERE wire.sc_track_id = lyrics.sc_track_id
            AND wire.status = 'pending'
      )
    FOR UPDATE
), claimed AS (
    UPDATE lyrics_cache AS lyrics
    SET embedding_state = 'queued'
    FROM candidate
    WHERE lyrics.sc_track_id = candidate.sc_track_id
    RETURNING lyrics.sc_track_id
)
SELECT sc_track_id AS "sc_track_id!"
FROM claimed
