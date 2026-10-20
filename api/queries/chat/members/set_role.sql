UPDATE chat.members
SET role = $3,
    updated_at = now()
WHERE conversation_id = $1
  AND user_id = $2
  AND state = 'active'
RETURNING user_id
