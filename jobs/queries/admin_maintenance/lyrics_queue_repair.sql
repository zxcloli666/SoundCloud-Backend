WITH locked AS MATERIALIZED (
    SELECT state.track_id,
           state.priority,
           state.next_run_at,
           state.created_at,
           CASE
               WHEN state.wake_message_id IS NOT NULL AND state.wake_durable_at IS NULL THEN state.updated_at
           END AS wake_at,
           state.attempts,
           state.last_attempt_at,
           state.claim_job_id,
           state.claim_job_generation,
           state.claim_job_lease_id,
           state.claim_state_generation,
           state.claim_expires_at
    FROM lyrics_lookup_state AS state
    WHERE state.track_id = ANY ($1::uuid[])
    ORDER BY state.track_id
    FOR SHARE OF state SKIP LOCKED
)
INSERT INTO lyrics_lookup_queue AS queue (
    track_id,
    priority,
    next_run_at,
    created_at,
    wake_at,
    attempts,
    last_attempt_at,
    claim_job_id,
    claim_job_generation,
    claim_job_lease_id,
    claim_state_generation,
    claim_expires_at
)
SELECT locked.track_id,
       locked.priority,
       locked.next_run_at,
       locked.created_at,
       locked.wake_at,
       locked.attempts,
       locked.last_attempt_at,
       locked.claim_job_id,
       locked.claim_job_generation,
       locked.claim_job_lease_id,
       locked.claim_state_generation,
       locked.claim_expires_at
FROM locked
ON CONFLICT (track_id) DO UPDATE
SET priority = EXCLUDED.priority,
    next_run_at = EXCLUDED.next_run_at,
    created_at = EXCLUDED.created_at,
    wake_at = EXCLUDED.wake_at,
    attempts = EXCLUDED.attempts,
    last_attempt_at = EXCLUDED.last_attempt_at,
    claim_job_id = EXCLUDED.claim_job_id,
    claim_job_generation = EXCLUDED.claim_job_generation,
    claim_job_lease_id = EXCLUDED.claim_job_lease_id,
    claim_state_generation = EXCLUDED.claim_state_generation,
    claim_expires_at = EXCLUDED.claim_expires_at
WHERE (
    queue.priority,
    queue.next_run_at,
    queue.created_at,
    queue.wake_at,
    queue.attempts,
    queue.last_attempt_at,
    queue.claim_job_id,
    queue.claim_job_generation,
    queue.claim_job_lease_id,
    queue.claim_state_generation,
    queue.claim_expires_at
) IS DISTINCT FROM (
    EXCLUDED.priority,
    EXCLUDED.next_run_at,
    EXCLUDED.created_at,
    EXCLUDED.wake_at,
    EXCLUDED.attempts,
    EXCLUDED.last_attempt_at,
    EXCLUDED.claim_job_id,
    EXCLUDED.claim_job_generation,
    EXCLUDED.claim_job_lease_id,
    EXCLUDED.claim_state_generation,
    EXCLUDED.claim_expires_at
)
