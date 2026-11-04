SELECT count(*) AS "filed!",
       count(*) FILTER (WHERE state = 'actioned') AS "upheld!",
       count(*) FILTER (WHERE state = 'rejected') AS "rejected!"
FROM chat.reports
WHERE reporter = $1
  AND created_at > now() - $2::int * interval '1 day'
