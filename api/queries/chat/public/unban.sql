UPDATE chat.members
SET state = 'active',
    updated_at = now()
WHERE conversation_id = $1
  AND user_id = $2
  AND state = 'banned'
RETURNING user_id
