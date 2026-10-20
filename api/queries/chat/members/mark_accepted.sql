UPDATE chat.members
SET accepted = true,
    updated_at = now()
WHERE conversation_id = $1
  AND user_id = $2
  AND state = 'active'
  AND NOT accepted
