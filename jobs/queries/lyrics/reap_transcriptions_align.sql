WITH plain AS MATERIALIZED (
    SELECT event.sc_track_id,
           event.uploaded_generation,
           event.updated_at,
           event.transcription_generation,
           wire.sc_track_id AS wire_track_id,
           wire.status AS wire_status,
           wire.quarantine_reason AS wire_quarantine_reason,
           wire.upload_generation AS wire_upload_generation
    FROM lyrics_cache AS lyrics
    JOIN storage_event_state AS event
      ON event.sc_track_id = lyrics.sc_track_id
    LEFT JOIN transcription_wire_state AS wire
      ON wire.sc_track_id = lyrics.sc_track_id
    WHERE NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
      AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
      AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
      AND event.updated_at < now() - interval '10 minutes'
      AND event.uploaded_generation > 0
      AND (
          (
              event.transcription_generation IS NULL
              AND wire.sc_track_id IS NULL
          )
          OR (
              wire.status = 'quarantined'
              AND wire.quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded')
              AND wire.upload_generation < event.uploaded_generation
          )
      )
)
SELECT track.sc_track_id AS "sc_track_id!",
       plain.uploaded_generation AS "uploaded_generation!",
       plain.updated_at AS "waiting_since!"
FROM plain
JOIN tracks AS track
  ON track.sc_track_id = plain.sc_track_id
WHERE track.storage_state = 'ok'
  AND NOT track.needs_duration_resolve
  AND track.sc_track_id <> ALL ($2::text[])
  AND (
      (
          plain.transcription_generation IS NULL
          AND track.transcribe_state IS NULL
          AND plain.wire_track_id IS NULL
      )
      OR (
          track.transcribe_state = 'quarantined'
          AND plain.wire_status = 'quarantined'
          AND plain.wire_quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded')
          AND plain.wire_upload_generation < plain.uploaded_generation
      )
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'lyrics.dispatch_transcription'
        AND job.dedup_key = track.sc_track_id
  )
ORDER BY plain.updated_at, plain.sc_track_id
FOR UPDATE OF track SKIP LOCKED
LIMIT $1
