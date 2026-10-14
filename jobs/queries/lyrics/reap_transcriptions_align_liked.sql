WITH window_end AS MATERIALIZED (
    SELECT event.updated_at
    FROM storage_event_state AS event
    WHERE event.uploaded_generation > 0
      AND event.transcription_generation IS NULL
    ORDER BY event.updated_at, event.sc_track_id
    OFFSET $4
    LIMIT 1
)
SELECT event.sc_track_id AS "sc_track_id!",
       event.uploaded_generation AS "uploaded_generation!",
       event.updated_at AS "waiting_since!"
FROM storage_event_state AS event
WHERE event.uploaded_generation > 0
  AND event.transcription_generation IS NULL
  AND event.updated_at <= coalesce((SELECT updated_at FROM window_end), 'infinity')
  AND event.updated_at < now() - interval '10 minutes'
  AND event.sc_track_id <> ALL ($3::text[])
  AND (
      SELECT track.index_priority = $2
             AND track.storage_state = 'ok'
             AND NOT track.needs_duration_resolve
             AND track.transcribe_state IS NULL
      FROM tracks AS track
      WHERE track.sc_track_id = event.sc_track_id
  )
  AND (
      SELECT NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
             AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
             AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
      FROM lyrics_cache AS lyrics
      WHERE lyrics.sc_track_id = event.sc_track_id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM transcription_wire_state AS wire
      WHERE wire.sc_track_id = event.sc_track_id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'lyrics.dispatch_transcription'
        AND job.dedup_key = event.sc_track_id
  )
ORDER BY event.updated_at, event.sc_track_id
FOR UPDATE OF event SKIP LOCKED
LIMIT $1
