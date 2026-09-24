SELECT cache.plain_text,
       cache.synced_lrc,
       cache.language,
       cache.content_generation,
       cache.embedding_state AS "embedding_state!",
       wire.request_message_id AS "request_id?",
       wire.request_text AS "request_text?",
       wire.request_language AS "request_language?",
       wire.publish_acknowledged_at IS NOT NULL AS "request_published?"
FROM lyrics_cache AS cache
LEFT JOIN lyrics_embedding_wire_state AS wire
  ON wire.sc_track_id = cache.sc_track_id
 AND wire.status = 'pending'
 AND wire.lyrics_content_generation = cache.content_generation
WHERE cache.sc_track_id = $1
  AND cache.embedded_at IS NULL
  AND cache.embedding_state IN ('queued', 'pending')
FOR UPDATE OF cache
