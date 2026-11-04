UPDATE chat.members
SET state = $3,
    updated_at = now()
WHERE conversation_id = $1
  AND user_id = $2
  AND state = ANY ($4)
RETURNING role
