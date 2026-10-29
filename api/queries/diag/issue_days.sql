SELECT date_trunc('day', at) AS "day!",
       count(*) AS "events!",
       count(DISTINCT install) AS "installs!"
FROM diag_events
WHERE at >= now() - make_interval(hours => $1)
  AND fingerprint = $2
GROUP BY 1
ORDER BY 1
