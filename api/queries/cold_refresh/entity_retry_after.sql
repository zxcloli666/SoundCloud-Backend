SELECT GREATEST(5, LEAST(1800, ceil(extract(epoch FROM available_at - now()))))::bigint AS "seconds!"
FROM background_jobs
WHERE kind = 'catalog.refresh' AND dedup_key = $1
