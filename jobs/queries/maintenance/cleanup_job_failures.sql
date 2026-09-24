WITH expired AS (
    SELECT failure.id, failure.generation
    FROM background_job_failures AS failure
    WHERE failure.failed_at < now() - interval '30 days'
    ORDER BY failure.failed_at
    FOR UPDATE OF failure SKIP LOCKED
    LIMIT 10_000
)
DELETE FROM background_job_failures AS failure
USING expired
WHERE failure.id = expired.id
  AND failure.generation = expired.generation
