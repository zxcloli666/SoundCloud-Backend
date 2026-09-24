WITH expired AS (
    SELECT receipt.id
    FROM background_job_enqueues AS receipt
    WHERE receipt.accepted_at < now() - interval '7 days'
      AND NOT EXISTS (
          SELECT 1
          FROM background_jobs AS job
          WHERE job.id = receipt.id
      )
      AND NOT EXISTS (
          SELECT 1
          FROM background_job_failures AS failure
          WHERE failure.id = receipt.id
      )
    ORDER BY receipt.accepted_at
    FOR UPDATE OF receipt SKIP LOCKED
    LIMIT 10_000
)
DELETE FROM background_job_enqueues AS receipt
USING expired
WHERE receipt.id = expired.id
