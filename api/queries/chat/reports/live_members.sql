SELECT user_id
FROM chat.members
WHERE conversation_id = $1
  AND state IN ('active', 'invited')
