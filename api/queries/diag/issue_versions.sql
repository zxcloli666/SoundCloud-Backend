SELECT fingerprint AS "fingerprint!",
       app_version AS "app_version!",
       count(*) AS "events!"
FROM diag_events
WHERE at >= now() - make_interval(hours => $1)
  AND fingerprint = ANY($2)
GROUP BY fingerprint, app_version
