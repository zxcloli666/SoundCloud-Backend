UPDATE audio_index_wire_state AS state
SET status = 'reopenable',
    outcome_rank = 1,
    outcome_status = 'failed',
    outcome_reason = 'dispatch_publish_failed',
    completed_at = now(),
    updated_at = now()
WHERE state.sc_track_id = $1
  AND state.status = 'pending'
  AND state.upload_generation = $2
  AND state.attempt = $3
  AND (
      state.result_lease_id IS NULL
      OR state.result_lease_expires_at <= now()
  )
