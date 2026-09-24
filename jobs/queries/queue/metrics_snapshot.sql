WITH lanes AS (
    SELECT lane FROM unnest($1::text[]) AS lane
), queued AS (
    SELECT lane,
           count(*) FILTER (WHERE lease_id IS NULL)::bigint AS pending,
           count(*) FILTER (WHERE lease_id IS NULL AND available_at <= now())::bigint AS due,
           count(*) FILTER (WHERE lease_id IS NOT NULL)::bigint AS leased,
           count(*) FILTER (WHERE lease_id IS NOT NULL AND lease_expires_at <= now())::bigint AS expired,
           count(*) FILTER (WHERE attempts > 0)::bigint AS retried,
           coalesce(
               max(extract(epoch FROM now() - available_at))
                   FILTER (WHERE lease_id IS NULL AND available_at <= now()),
               0
           )::float8 AS oldest_due_seconds
    FROM background_jobs
    GROUP BY lane
), failures AS (
    SELECT lane, count(*)::bigint AS dead_letters
    FROM background_job_failures
    GROUP BY lane
)
SELECT lanes.lane AS "lane!",
       coalesce(queued.pending, 0) AS "pending!",
       coalesce(queued.due, 0) AS "due!",
       coalesce(queued.leased, 0) AS "leased!",
       coalesce(queued.expired, 0) AS "expired!",
       coalesce(queued.retried, 0) AS "retried!",
       coalesce(queued.oldest_due_seconds, 0) AS "oldest_due_seconds!",
       coalesce(failures.dead_letters, 0) AS "dead_letters!"
FROM lanes
LEFT JOIN queued ON queued.lane = lanes.lane
LEFT JOIN failures ON failures.lane = lanes.lane
