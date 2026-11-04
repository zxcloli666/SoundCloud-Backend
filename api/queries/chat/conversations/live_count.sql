SELECT count(*) AS "count!"
FROM chat.members
WHERE user_id = $1
  AND state IN ('active', 'invited')
