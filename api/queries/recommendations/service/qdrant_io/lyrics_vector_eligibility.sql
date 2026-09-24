SELECT cache.sc_track_id,
       wire.request_message_id AS "request_id!"
FROM lyrics_cache AS cache
JOIN lyrics_embedding_wire_state AS wire
  ON wire.sc_track_id = cache.sc_track_id
WHERE cache.sc_track_id = ANY($1)
  AND cache.embedded_at IS NOT NULL
  AND cache.embedding_state = 'done'
  AND wire.request_message_id IS NOT NULL
  AND wire.lyrics_created_at = cache.created_at
