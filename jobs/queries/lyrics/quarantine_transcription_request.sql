WITH quarantined AS (
    UPDATE transcription_wire_state AS state
    SET status = 'quarantined',
        quarantine_reason = $4::varchar,
        completed_at = now(),
        updated_at = now()
    WHERE state.sc_track_id = $1
      AND state.upload_generation = $2
      AND state.attempt = $3
      AND state.status = 'pending'
    RETURNING state.sc_track_id
)
UPDATE tracks AS track
SET transcribe_state = 'quarantined',
    transcribe_at = now(),
    updated_at = now()
FROM quarantined
WHERE track.sc_track_id = quarantined.sc_track_id
