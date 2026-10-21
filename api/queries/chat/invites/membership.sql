SELECT state
FROM chat.members
WHERE conversation_id = $1
  AND user_id = $2
