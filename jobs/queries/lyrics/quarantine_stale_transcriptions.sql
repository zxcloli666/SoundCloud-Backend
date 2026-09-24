WITH candidates AS MATERIALIZED (
    SELECT track.sc_track_id
    FROM transcription_wire_state AS wire
    JOIN tracks AS track
      ON track.sc_track_id = wire.sc_track_id
    WHERE wire.status = 'pending'
      AND wire.dispatched_at < now() - $2::bigint * interval '1 second'
      AND track.transcribe_state = 'pending'
    ORDER BY wire.dispatched_at, track.sc_track_id
    FOR UPDATE OF track SKIP LOCKED
    LIMIT $1
), locked_wire AS MATERIALIZED (
    SELECT wire.sc_track_id
    FROM transcription_wire_state AS wire
    JOIN candidates
      ON candidates.sc_track_id = wire.sc_track_id
    WHERE wire.status = 'pending'
    FOR UPDATE OF wire
), quarantined AS (
    UPDATE transcription_wire_state AS wire
    SET status = 'quarantined',
        completed_at = now(),
        quarantine_reason = 'result_timeout',
        updated_at = now()
    FROM locked_wire
    WHERE wire.sc_track_id = locked_wire.sc_track_id
    RETURNING wire.sc_track_id
), updated AS (
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
FROM updated
