SELECT lane AS "lane!",
       kind AS "kind!",
       count(*) FILTER (WHERE lease_id IS NULL)::bigint AS "pending!",
       count(*) FILTER (WHERE lease_id IS NULL AND available_at <= now())::bigint AS "due!",
       count(*) FILTER (WHERE lease_id IS NOT NULL)::bigint AS "leased!",
       coalesce(
           max(extract(epoch FROM now() - available_at))
               FILTER (WHERE lease_id IS NULL AND available_at <= now()),
           0
       )::float8 AS "oldest_due_seconds!",
       coalesce(max(extract(epoch FROM now() - created_at)), 0)::float8 AS "oldest_age_seconds!"
FROM background_jobs
GROUP BY lane, kind
