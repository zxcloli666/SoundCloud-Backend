SELECT GREATEST(5, LEAST(1800, ceil(extract(epoch FROM COALESCE(available_at, now()) - now()))))::bigint AS "seconds!"
FROM job_status
WHERE kind = 'catalog.refresh' AND dedup_key = $1
