SELECT user_id
FROM chat.members
WHERE conversation_id = $1
  AND user_id = ANY($2)
  AND state = 'active'
