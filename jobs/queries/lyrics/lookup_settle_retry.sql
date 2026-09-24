UPDATE lyrics_lookup_state AS state
SET status = 'retry',
    next_run_at = now() + $6::bigint * interval '1 second',
    failure_streak = state.failure_streak + 1,
    last_outcome = $7,
    last_error = left($8, 512),
    retry_after_at = CASE
        WHEN $9::bigint > 0 THEN now() + $9::bigint * interval '1 second'
        ELSE NULL
    END,
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
