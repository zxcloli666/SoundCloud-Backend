SELECT conversation_id, user_id
FROM chat.members
WHERE conversation_id = ANY($1)
  AND state IN ('active', 'invited')
