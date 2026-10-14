SELECT track.sc_track_id AS "sc_track_id!",
       event.uploaded_generation AS "uploaded_generation!",
       event.updated_at AS "waiting_since!"
FROM star_track_tiers AS star
JOIN storage_event_state AS event
  ON event.sc_track_id = star.sc_track_id
JOIN tracks AS track
  ON track.sc_track_id = event.sc_track_id
JOIN lyrics_cache AS lyrics
  ON lyrics.sc_track_id = track.sc_track_id
LEFT JOIN transcription_wire_state AS wire
  ON wire.sc_track_id = track.sc_track_id
WHERE NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
  AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
  AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
  AND event.updated_at < now() - interval '10 minutes'
  AND event.uploaded_generation > 0
  AND track.storage_state = 'ok'
  AND NOT track.needs_duration_resolve
  AND star.tier = $2
  AND track.sc_track_id <> ALL ($3::text[])
  AND (
      (
          event.transcription_generation IS NULL
          AND track.transcribe_state IS NULL
          AND wire.sc_track_id IS NULL
      )
      OR (
          track.transcribe_state = 'quarantined'
          AND wire.status = 'quarantined'
          AND wire.quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded')
          AND wire.upload_generation < event.uploaded_generation
      )
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'lyrics.dispatch_transcription'
        AND job.dedup_key = track.sc_track_id
  )
ORDER BY star.tiered_at, star.sc_track_id
FOR UPDATE OF track SKIP LOCKED
LIMIT $1
