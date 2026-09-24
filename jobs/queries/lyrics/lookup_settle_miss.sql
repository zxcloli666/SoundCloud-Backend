UPDATE lyrics_lookup_state AS state
SET status = 'not_found',
    next_run_at = now() + $6::bigint * interval '1 second',
    miss_streak = state.miss_streak + 1,
    failure_streak = 0,
    last_outcome = $7,
    last_error = NULL,
    retry_after_at = NULL,
    wake_message_id = NULL,
    wake_generation = NULL,
    wake_durable_at = NULL,
    claim_job_id = NULL,
    claim_job_generation = NULL,
    claim_job_lease_id = NULL,
    claim_state_generation = NULL,
    claim_expires_at = NULL,
    priority = COALESCE((SELECT track.index_priority FROM tracks AS track WHERE track.id = state.track_id), state.priority),
    updated_at = now()
WHERE state.track_id = $4
  AND state.generation = $5
  AND state.claim_job_id = $1
  AND state.claim_job_generation = $2
  AND state.claim_job_lease_id = $3
  AND state.claim_state_generation = $5
  AND state.claim_expires_at > now()
  AND EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.id = $1
        AND job.generation = $2
        AND job.lease_generation = $2
        AND job.lease_id = $3
        AND job.lease_expires_at > now()
  )
RETURNING state.track_id
