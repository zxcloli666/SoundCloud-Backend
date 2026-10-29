SELECT id.sc_track_id AS "sc_track_id!",
       (track.sc_track_id IS NOT NULL) AS "is_public!",
       wire.request_message_id AS "request_id?"
FROM unnest($1::text[]) AS id (sc_track_id)
LEFT JOIN tracks AS track
  ON track.sc_track_id = id.sc_track_id
 AND track.sharing = 'public'
 AND track.superseded_by IS NULL
 AND track.deleted_at IS NULL
 AND track.index_state = 'indexed'
 AND track.storage_state <> 'too_long'
 AND track.needs_duration_resolve = false
LEFT JOIN lyrics_cache AS cache
  ON cache.sc_track_id = id.sc_track_id
 AND cache.embedded_at IS NOT NULL
 AND cache.embedding_state = 'done'
LEFT JOIN lyrics_embedding_wire_state AS wire
  ON wire.sc_track_id = cache.sc_track_id
 AND wire.request_message_id IS NOT NULL
 AND wire.lyrics_created_at = cache.created_at
WHERE track.sc_track_id IS NOT NULL
   OR wire.sc_track_id IS NOT NULL
