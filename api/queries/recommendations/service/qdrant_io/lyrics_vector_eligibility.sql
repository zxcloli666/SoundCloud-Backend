SELECT cache.sc_track_id,
       cache.embedding_request_id AS "request_id!"
FROM lyrics_cache AS cache
WHERE cache.sc_track_id = ANY($1)
  AND cache.embedded_at IS NOT NULL
  AND cache.embedding_state = 'done'
  AND cache.embedding_request_id IS NOT NULL
  AND cache.embedding_request_created_at = cache.created_at
