SELECT updated_at AS "queued_since!",
       GREATEST(5, LEAST(1800, ceil(extract(epoch FROM COALESCE(available_at, now()) - now()))))::bigint AS "retry_after_seconds!"
FROM job_status
WHERE kind = 'catalog.collection' AND dedup_key = $1
