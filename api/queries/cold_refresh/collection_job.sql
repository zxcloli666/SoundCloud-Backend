SELECT created_at,
       GREATEST(5, LEAST(1800, ceil(extract(epoch FROM available_at - now()))))::bigint AS "retry_after_seconds!"
FROM background_jobs
WHERE kind = 'catalog.collection' AND dedup_key = $1
