WITH due AS (
    SELECT wire.sc_track_id
    FROM audio_index_wire_state AS wire
    JOIN tracks AS track ON track.sc_track_id = wire.sc_track_id
    JOIN storage_event_state AS storage
      ON storage.sc_track_id = wire.sc_track_id
     AND storage.uploaded_generation = wire.upload_generation
    WHERE (
            wire.status = 'reopenable'
            OR (wire.status = 'terminal' AND wire.outcome_reason = 'audio_forbidden')
        )
      AND wire.updated_at < now() - $2::bigint * interval '1 second'
      AND wire.attempt < $3
      AND (
          wire.result_lease_id IS NULL
          OR wire.result_lease_expires_at <= now()
      )
      AND track.storage_state = 'ok'
      AND track.index_state NOT IN ('indexed', 'too_long')
      AND NOT track.needs_duration_resolve
    ORDER BY wire.updated_at, wire.sc_track_id
    LIMIT $1
    FOR UPDATE OF wire SKIP LOCKED
)
UPDATE audio_index_wire_state AS wire
SET status = 'pending',
    attempt = wire.attempt + 1,
    outcome_rank = 0,
    outcome_status = NULL,
    outcome_reason = NULL,
    dispatched_at = now(),
    completed_at = NULL,
    result_consumer = NULL,
    result_stream = NULL,
    result_stream_sequence = NULL,
    result_published_at = NULL,
    result_lease_id = NULL,
    result_lease_expires_at = NULL,
    updated_at = now()
FROM due
WHERE wire.sc_track_id = due.sc_track_id
RETURNING wire.sc_track_id,
          wire.upload_generation AS "upload_generation!",
          wire.attempt
