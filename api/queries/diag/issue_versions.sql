SELECT fingerprint AS "fingerprint!",
       app_version AS "app_version!",
       sum(events)::bigint AS "events!"
FROM diag_issue_days
WHERE day >= $1
  AND fingerprint = ANY($2)
GROUP BY fingerprint, app_version
