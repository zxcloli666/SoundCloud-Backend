WITH expired AS (
    SELECT id
    FROM background_jobs
    WHERE attempts < max_attempts
      AND lease_id IS NOT NULL
      AND lease_expires_at <= now()
      AND kind = ANY($2::text[])
      AND lane = $3
    ORDER BY lease_expires_at, created_at, id
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
UPDATE background_jobs AS job
SET lease_id = NULL,
    lease_generation = NULL,
    leased_by = NULL,
    lease_expires_at = NULL,
    available_at = LEAST(job.available_at, now()),
    updated_at = now()
FROM expired
WHERE job.id = expired.id
