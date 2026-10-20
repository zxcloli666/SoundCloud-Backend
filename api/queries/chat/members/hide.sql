UPDATE chat.members m
SET hidden_before_seq = c.last_seq,
    updated_at = now()
FROM chat.conversations c
WHERE m.conversation_id = $1
  AND m.user_id = $2
  AND m.state IN ('active', 'invited')
  AND c.id = m.conversation_id
RETURNING m.hidden_before_seq
