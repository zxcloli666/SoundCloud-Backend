UPDATE lyrics_cache AS cache
SET embedding_state = NULL
WHERE cache.sc_track_id = $1
  AND cache.embedded_at IS NULL
  AND (
      cache.embedding_state = 'queued'
      OR (
          cache.embedding_state = 'pending'
          AND NOT EXISTS (
              SELECT 1
              FROM lyrics_embedding_wire_state AS wire
              WHERE wire.sc_track_id = cache.sc_track_id
                AND wire.status = 'pending'
          )
      )
  )
