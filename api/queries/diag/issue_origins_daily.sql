SELECT day AS "day!",
       count(DISTINCT origin) AS "installs!"
FROM diag_issue_origins
WHERE day >= $1
  AND fingerprint = $2
GROUP BY day
