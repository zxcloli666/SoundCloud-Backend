WITH expired AS MATERIALIZED (
    SELECT sc_track_id
    FROM lyrics_repair
    WHERE kind = 'refetch'
      AND claim_expires_at <= now()
      AND claim_job_id IS NOT NULL
    ORDER BY claim_expires_at, sc_track_id
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
UPDATE lyrics_repair AS repair
SET status = CASE WHEN repair.attempts >= $2 THEN 'done' ELSE 'pending' END,
    outcome = CASE WHEN repair.attempts >= $2 THEN 'abandoned' END,
    settled_at = CASE WHEN repair.attempts >= $2 THEN now() END,
    claim_job_id = NULL,
    claim_job_generation = NULL,
    claim_job_lease_id = NULL,
    claim_expires_at = NULL,
    updated_at = now()
FROM expired
WHERE repair.sc_track_id = expired.sc_track_id
RETURNING repair.status AS "status!"
