SELECT sc_user_id
FROM artist_sc_accounts
WHERE artist_id = $1
  AND role IN ('main', 'alt', 'demo')
  AND (verified OR source = 'mb_resolve')
ORDER BY verified DESC,
         CASE role
             WHEN 'main' THEN 0
             WHEN 'demo' THEN 1
             WHEN 'alt' THEN 2
             ELSE 3
             END
