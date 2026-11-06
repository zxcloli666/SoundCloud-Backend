UPDATE chat.members
SET last_ping_seq = $3
WHERE conversation_id = $1
  AND user_id = ANY($2)
  AND state = 'active'
  AND last_ping_seq < $3
RETURNING user_id
