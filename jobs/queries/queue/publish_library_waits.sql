WITH due AS (
    SELECT job.kind,
           CASE WHEN job.priority >= $3::smallint THEN 'read' ELSE 'queue' END AS lane,
           job.available_at
    FROM background_jobs AS job
    WHERE job.lane = $2
      AND job.kind = ANY($1::text[])
      AND job.priority >= $4::smallint
      AND job.lease_id IS NULL
      AND job.attempts < job.max_attempts
      AND job.available_at <= now()
)
INSERT INTO job_queue_waits AS wait (kind, lane, due, oldest_due_at, updated_at)
SELECT kinds.kind, lanes.lane, count(due.kind), min(due.available_at), now()
FROM unnest($1::text[]) AS kinds(kind)
CROSS JOIN (VALUES ('read'), ('queue')) AS lanes(lane)
LEFT JOIN due ON due.kind = kinds.kind AND due.lane = lanes.lane
GROUP BY kinds.kind, lanes.lane
ON CONFLICT (kind, lane) DO UPDATE
SET due = EXCLUDED.due,
    oldest_due_at = EXCLUDED.oldest_due_at,
    updated_at = EXCLUDED.updated_at
RETURNING wait.kind AS "kind!",
          wait.lane AS "lane!",
          wait.due AS "due!",
          coalesce(extract(epoch FROM now() - wait.oldest_due_at), 0)::float8 AS "waiting_seconds!"
