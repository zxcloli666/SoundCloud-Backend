WITH owned AS MATERIALIZED (
    SELECT state.track_id,
           state.generation = $5 AS current
    FROM lyrics_lookup_state AS state
    JOIN background_jobs AS job
      ON job.id = $1
     AND job.generation = $2
     AND job.lease_generation = $2
     AND job.lease_id = $3
     AND job.lease_expires_at > now()
    WHERE state.track_id = $4
    FOR UPDATE OF state
), claim AS MATERIALIZED (
    SELECT owned.track_id, owned.current
    FROM owned
    JOIN lyrics_lookup_queue AS queue
      ON queue.track_id = owned.track_id
    WHERE queue.claim_job_id = $1
      AND queue.claim_job_generation = $2
      AND queue.claim_job_lease_id = $3
      AND queue.claim_state_generation = $5
      AND queue.claim_expires_at > now()
    FOR UPDATE OF queue
), settled AS (
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
        priority = COALESCE((SELECT track.index_priority FROM tracks AS track WHERE track.id = state.track_id), state.priority),
        updated_at = now()
    FROM claim
    WHERE state.track_id = claim.track_id
      AND claim.current
    RETURNING state.track_id, state.priority, state.next_run_at
), released AS (
    UPDATE lyrics_lookup_queue AS queue
    SET priority = COALESCE(settled.priority, queue.priority),
        next_run_at = COALESCE(settled.next_run_at, queue.next_run_at),
        wake_at = CASE WHEN settled.track_id IS NULL THEN queue.wake_at END,
        claim_job_id = NULL,
        claim_job_generation = NULL,
        claim_job_lease_id = NULL,
        claim_state_generation = NULL,
        claim_expires_at = NULL
    FROM claim
    LEFT JOIN settled ON settled.track_id = claim.track_id
    WHERE queue.track_id = claim.track_id
    RETURNING queue.track_id
)
SELECT settled.track_id AS "track_id!"
FROM settled
