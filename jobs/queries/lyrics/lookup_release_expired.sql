WITH expired AS MATERIALIZED (
    SELECT track_id
    FROM lyrics_lookup_queue
    WHERE claim_expires_at <= now()
      AND claim_job_id IS NOT NULL
    ORDER BY claim_expires_at, track_id
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
UPDATE lyrics_lookup_queue AS queue
SET claim_job_id = NULL,
    claim_job_generation = NULL,
    claim_job_lease_id = NULL,
    claim_state_generation = NULL,
    claim_expires_at = NULL
FROM expired
WHERE queue.track_id = expired.track_id
