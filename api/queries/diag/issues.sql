WITH busiest AS (
    SELECT fingerprint
    FROM diag_issue_days
    WHERE day >= $1
      AND ($3::text IS NULL OR fingerprint = $3)
    GROUP BY fingerprint
    ORDER BY sum(events) DESC
    LIMIT 1000
),
grouped AS (
    SELECT fingerprint,
           (array_agg(kind ORDER BY first_at))[1] AS kind,
           (array_agg(title ORDER BY first_at))[1] AS title,
           (array_agg(app_version ORDER BY first_at))[1] AS first_version,
           sum(events)::bigint AS events,
           min(first_at) AS first_at,
           max(last_at) AS last_at
    FROM diag_issue_days
    WHERE day >= $1
      AND fingerprint IN (SELECT fingerprint FROM busiest)
    GROUP BY fingerprint
),
installs AS (
    SELECT fingerprint, count(DISTINCT origin) AS installs
    FROM diag_issue_origins
    WHERE day >= $1
      AND fingerprint IN (SELECT fingerprint FROM busiest)
    GROUP BY fingerprint
)
SELECT grouped.fingerprint AS "fingerprint!",
       grouped.kind AS "kind!",
       grouped.title AS "title!",
       grouped.events AS "events!",
       coalesce(installs.installs, 0) AS "installs!",
       grouped.first_at AS "first_at!",
       grouped.last_at AS "last_at!",
       grouped.first_version AS "first_version!"
FROM grouped
LEFT JOIN installs USING (fingerprint)
ORDER BY 5 DESC, grouped.events DESC
LIMIT $2
