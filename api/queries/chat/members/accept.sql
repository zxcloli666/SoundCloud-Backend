UPDATE chat.members
SET state = 'active',
    accepted = true,
    updated_at = now()
WHERE conversation_id = $1
  AND user_id = $2
  AND state = ANY ($3)
RETURNING user_id
