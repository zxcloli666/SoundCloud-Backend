SELECT due AS "ahead!",
       GREATEST(0, ceil(extract(epoch FROM now() - coalesce(oldest_due_at, now()))))::bigint AS "eta_seconds!"
FROM job_queue_waits
WHERE kind = $1
  AND lane = $2
  AND updated_at > now() - interval '5 minutes'
