WITH acknowledged AS (
    UPDATE lyrics_embedding_wire_state AS wire
    SET publish_acknowledged_at = COALESCE(wire.publish_acknowledged_at, now()),
        updated_at = now()
    WHERE wire.sc_track_id = $1
      AND wire.request_message_id = $2
      AND wire.status = 'pending'
    RETURNING wire.sc_track_id
)
UPDATE lyrics_cache AS cache
SET embedding_state = 'dispatched'
FROM acknowledged
WHERE cache.sc_track_id = acknowledged.sc_track_id
  AND cache.embedding_state = 'pending'
