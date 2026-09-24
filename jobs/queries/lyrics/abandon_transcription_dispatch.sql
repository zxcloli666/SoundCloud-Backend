UPDATE transcription_wire_state AS state
SET status = 'reopenable',
    reason = 'dispatch_publish_failed',
    completed_at = now(),
    updated_at = now()
WHERE state.sc_track_id = $1
  AND state.status = 'pending'
  AND state.upload_generation = $2
  AND state.attempt = $3
  AND state.result_stream_sequence IS NULL
