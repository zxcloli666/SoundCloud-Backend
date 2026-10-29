SELECT fingerprint AS "fingerprint!",
       (array_agg(kind ORDER BY at DESC))[1] AS "kind!",
       (array_agg(title ORDER BY at DESC))[1] AS "title!",
       count(*) AS "events!",
       count(DISTINCT install) AS "installs!",
       min(at) AS "first_at!",
       max(at) AS "last_at!",
       (array_agg(app_version ORDER BY at))[1] AS "first_version!"
FROM diag_events
WHERE at >= now() - make_interval(hours => $1)
  AND ($3::text IS NULL OR fingerprint = $3)
GROUP BY fingerprint
ORDER BY count(DISTINCT install) DESC, count(*) DESC
LIMIT $2
