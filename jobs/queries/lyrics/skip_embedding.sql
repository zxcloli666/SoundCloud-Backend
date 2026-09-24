UPDATE lyrics_cache AS cache
SET embedding_state = 'skipped'
WHERE cache.sc_track_id = $1
  AND cache.content_generation = $2
  AND cache.embedded_at IS NULL
  AND cache.embedding_state = 'queued'
