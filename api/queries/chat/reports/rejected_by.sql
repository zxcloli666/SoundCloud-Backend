SELECT count(*) AS "rejected!"
FROM chat.reports
WHERE reporter = $1
  AND state = 'rejected'
  AND handled_at > now() - $2::int * interval '1 day'
