UPDATE chat.members
SET role = 'admin',
    updated_at = now()
WHERE conversation_id = $1
  AND user_id = $2
  AND role = 'owner'
  AND state = 'active'
RETURNING user_id
