WITH candidates AS MATERIALIZED (
    SELECT wire.sc_track_id,
           storage.uploaded_generation IS NOT DISTINCT FROM wire.upload_generation AS current
    FROM audio_index_wire_state AS wire
    LEFT JOIN storage_event_state AS storage
      ON storage.sc_track_id = wire.sc_track_id
    WHERE (
            wire.status = 'reopenable'
            OR (wire.status = 'terminal' AND wire.outcome_reason = 'audio_forbidden')
        )
      AND wire.updated_at < now() - $2::bigint * interval '1 second'
      AND (
          wire.attempt >= $3
          OR storage.uploaded_generation IS DISTINCT FROM wire.upload_generation
      )
      AND (
          wire.result_lease_id IS NULL
          OR wire.result_lease_expires_at <= now()
      )
    ORDER BY wire.updated_at, wire.sc_track_id
    LIMIT $1
    FOR UPDATE OF wire SKIP LOCKED
), quarantined AS (
    UPDATE audio_index_wire_state AS wire
    SET status = 'quarantined',
        quarantine_reason = CASE
            WHEN candidates.current THEN 'reopen_attempts_exhausted'
            ELSE 'reopen_superseded'
        END,
        completed_at = now(),
        result_lease_id = NULL,
        result_lease_expires_at = NULL,
        updated_at = now()
    FROM candidates
    WHERE wire.sc_track_id = candidates.sc_track_id
    RETURNING wire.sc_track_id,
              candidates.current
), failed AS (
    UPDATE tracks AS track
    SET index_state = 'failed',
        updated_at = now()
    FROM quarantined
    WHERE track.sc_track_id = quarantined.sc_track_id
      AND quarantined.current
      AND track.index_state = 'pending'
    RETURNING track.sc_track_id
)
SELECT count(*)::bigint AS "quarantined!"
FROM quarantined
