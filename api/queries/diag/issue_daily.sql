SELECT fingerprint AS "fingerprint!",
       day AS "day!",
       sum(events)::bigint AS "events!"
FROM diag_issue_days
WHERE day >= $1
  AND fingerprint = ANY($2)
GROUP BY fingerprint, day
