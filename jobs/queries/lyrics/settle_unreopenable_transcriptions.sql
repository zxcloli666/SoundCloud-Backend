WITH candidates AS MATERIALIZED (
    SELECT wire.sc_track_id,
           CASE
               WHEN wire.reopen_count >= $2::integer THEN 'reopen_attempts_exhausted'
               ELSE 'reopen_superseded'
           END AS quarantine_reason
    FROM transcription_wire_state AS wire
    LEFT JOIN tracks AS track
      ON track.sc_track_id = wire.sc_track_id
    LEFT JOIN storage_event_state AS event
      ON event.sc_track_id = wire.sc_track_id
    LEFT JOIN lyrics_cache AS lyrics
      ON lyrics.sc_track_id = wire.sc_track_id
    WHERE wire.status = 'reopenable'
      AND wire.completed_at < now() - $1::bigint * interval '1 second'
      AND (
          wire.reopen_count >= $2::integer
          OR track.sc_track_id IS NULL
          OR event.uploaded_generation IS DISTINCT FROM wire.upload_generation
          OR track.storage_state <> 'ok'
          OR track.needs_duration_resolve
          OR NULLIF(btrim(lyrics.plain_text), '') IS NULL
          OR NULLIF(btrim(lyrics.synced_lrc), '') IS NOT NULL
      )
    ORDER BY wire.completed_at, wire.sc_track_id
    FOR UPDATE OF wire SKIP LOCKED
    LIMIT $3
), quarantined AS (
    UPDATE transcription_wire_state AS wire
    SET status = 'quarantined',
        quarantine_reason = candidates.quarantine_reason,
        completed_at = now(),
        updated_at = now()
    FROM candidates
    WHERE wire.sc_track_id = candidates.sc_track_id
    RETURNING wire.sc_track_id
), tracked AS (
    UPDATE tracks AS track
    SET transcribe_state = 'quarantined',
        transcribe_at = now(),
        updated_at = now()
    FROM quarantined
    WHERE track.sc_track_id = quarantined.sc_track_id
      AND track.transcribe_state = 'pending'
    RETURNING track.sc_track_id
)
SELECT count(*)::bigint AS "quarantined!"
FROM quarantined
