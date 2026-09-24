UPDATE transcription_wire_state AS state
SET status = 'reopenable',
    reason = $4::varchar,
    quarantine_reason = NULL,
    result_rank = $5::smallint,
    completed_at = now(),
    updated_at = now()
FROM storage_event_state AS storage
WHERE state.sc_track_id = $1
  AND storage.sc_track_id = state.sc_track_id
  AND storage.uploaded_generation = $2
  AND state.upload_generation = $2
  AND state.attempt = $3
  AND state.status IN ('pending', 'reopenable')
  AND COALESCE(state.result_rank, 0) <= $5::smallint
RETURNING state.sc_track_id AS "sc_track_id!"
