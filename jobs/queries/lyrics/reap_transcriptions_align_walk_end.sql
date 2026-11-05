SELECT event.updated_at AS "updated_at!",
       event.sc_track_id AS "sc_track_id!"
FROM storage_event_state AS event
WHERE event.uploaded_generation > 0
  AND event.transcription_generation IS NULL
  AND event.updated_at < now() - interval '10 minutes'
  AND (event.updated_at, event.sc_track_id)
      > (coalesce($1, '-infinity'::timestamptz), coalesce($2, ''))
ORDER BY event.updated_at, event.sc_track_id
OFFSET $3
LIMIT 1
