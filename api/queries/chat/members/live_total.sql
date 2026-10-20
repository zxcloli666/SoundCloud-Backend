SELECT count(*) AS "count!"
FROM chat.members
WHERE conversation_id = $1
  AND state IN ('active', 'invited')
