SET LOCAL lock_timeout = '5s';

WITH released AS (
    UPDATE transcription_wire_state
    SET status = 'reopenable',
        reason = 'result_timeout',
        quarantine_reason = NULL,
        updated_at = now()
    WHERE status = 'quarantined'
      AND quarantine_reason = 'result_timeout'
      AND upload_generation IS NOT NULL
      AND completed_at IS NOT NULL
    RETURNING sc_track_id
)
UPDATE tracks AS track
SET transcribe_state = 'pending',
    updated_at = now()
FROM released
WHERE track.sc_track_id = released.sc_track_id
  AND track.transcribe_state = 'quarantined';
