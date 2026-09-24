WITH candidates AS (
    SELECT id, generation
    FROM background_jobs
    WHERE attempts >= max_attempts
      AND (lease_id IS NULL OR lease_expires_at <= now())
    ORDER BY lease_expires_at NULLS FIRST, created_at
    FOR UPDATE SKIP LOCKED
    LIMIT $1
), exhausted AS (
    DELETE FROM background_jobs AS job
    USING candidates
    WHERE job.id = candidates.id
      AND job.generation = candidates.generation
    RETURNING job.id,
              job.kind,
              job.lane,
              job.dedup_key,
              job.payload,
              job.priority,
              job.generation,
              job.attempts,
              job.max_attempts,
              CASE
                  WHEN job.lease_id IS NOT NULL
                      THEN 'lease expired after final attempt'
                  ELSE COALESCE(job.last_error, 'attempts exhausted')
              END AS last_error,
              job.created_at
)
INSERT INTO background_job_failures (
    id,
    kind,
    lane,
    dedup_key,
    payload,
    priority,
    generation,
    attempts,
    max_attempts,
    last_error,
    created_at,
    failed_at
)
SELECT id,
       kind,
       lane,
       dedup_key,
       payload,
       priority,
       generation,
       attempts,
       max_attempts,
       last_error,
       created_at,
       now()
FROM exhausted
ON CONFLICT (id, generation) DO UPDATE SET
    kind = EXCLUDED.kind,
    lane = EXCLUDED.lane,
    dedup_key = EXCLUDED.dedup_key,
    payload = EXCLUDED.payload,
    priority = EXCLUDED.priority,
    attempts = EXCLUDED.attempts,
    max_attempts = EXCLUDED.max_attempts,
    last_error = EXCLUDED.last_error,
    created_at = EXCLUDED.created_at,
    failed_at = EXCLUDED.failed_at
