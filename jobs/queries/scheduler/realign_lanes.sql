WITH expected AS (
    SELECT kind, lane
    FROM unnest($1::text[], $2::text[]) AS expected(kind, lane)
), stale AS (
    SELECT job.id, expected.lane
    FROM background_jobs AS job
    JOIN expected ON expected.kind = job.kind
    WHERE job.lane <> expected.lane
      AND job.lease_id IS NULL
    LIMIT $3
    FOR UPDATE OF job SKIP LOCKED
)
UPDATE background_jobs AS job
SET lane = stale.lane,
    updated_at = now()
FROM stale
WHERE job.id = stale.id
  AND job.lease_id IS NULL
