SELECT count(*) AS "reported!",
       count(*) FILTER (WHERE state = 'actioned') AS "upheld!",
       EXISTS (
           SELECT 1
           FROM chat.users
           WHERE user_id = $1
             AND restricted_until > now()
       ) AS "restricted!"
FROM chat.reports
WHERE target_user = $1
  AND created_at > now() - $2::int * interval '1 day'
