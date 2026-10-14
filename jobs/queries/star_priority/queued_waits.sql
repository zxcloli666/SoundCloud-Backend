SELECT wanted.kind AS "kind!",
       wanted.priority AS "priority!",
       coalesce(extract(epoch FROM now() - head.available_at), 0)::float8 AS "waiting_seconds!"
FROM unnest($1::text[], $2::smallint[]) AS wanted(kind, priority)
LEFT JOIN LATERAL (
    SELECT job.available_at
    FROM background_jobs AS job
    WHERE job.kind = wanted.kind
      AND job.priority = wanted.priority
      AND job.lease_id IS NULL
      AND job.attempts < job.max_attempts
      AND job.available_at <= now()
    ORDER BY job.available_at
    LIMIT 1
) AS head ON true
