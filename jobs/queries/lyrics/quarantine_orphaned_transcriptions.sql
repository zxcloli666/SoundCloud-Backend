WITH candidates AS MATERIALIZED (
    SELECT wire.sc_track_id
    FROM transcription_wire_state AS wire
    WHERE wire.status = 'pending'
      AND wire.dispatched_at < now() - $2::bigint * interval '1 second'
      AND NOT EXISTS (
          SELECT 1
          FROM tracks AS track
          WHERE track.sc_track_id = wire.sc_track_id
      )
    ORDER BY wire.dispatched_at, wire.sc_track_id
    FOR UPDATE OF wire SKIP LOCKED
    LIMIT $1
), quarantined AS (
    UPDATE transcription_wire_state AS wire
    SET status = 'quarantined',
        completed_at = now(),
        quarantine_reason = 'track_missing_after_result_timeout',
        updated_at = now()
    FROM candidates
    WHERE wire.sc_track_id = candidates.sc_track_id
    RETURNING wire.sc_track_id
)
SELECT count(*)::bigint AS "quarantined!"
FROM quarantined
