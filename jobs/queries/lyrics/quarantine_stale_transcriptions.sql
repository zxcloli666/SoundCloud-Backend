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
), released AS (
    UPDATE transcription_wire_state AS wire
    SET status = 'reopenable',
        reason = 'result_timeout',
        quarantine_reason = NULL,
        completed_at = now(),
        result_timeouts = wire.result_timeouts + 1,
        updated_at = now()
    FROM locked_wire
    WHERE wire.sc_track_id = locked_wire.sc_track_id
    RETURNING wire.sc_track_id,
              wire.result_timeouts
)
SELECT count(*)::bigint AS "released!",
       COALESCE(
           array_agg(sc_track_id ORDER BY sc_track_id)
               FILTER (WHERE result_timeouts >= $3::integer),
           ARRAY[]::text[]
       ) AS "repeated!"
FROM released
