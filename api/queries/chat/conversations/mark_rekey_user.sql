UPDATE chat.conversations c
SET rekey = true
FROM chat.members m
WHERE m.user_id = $1
  AND m.state IN ('active', 'invited')
  AND c.id = m.conversation_id
  AND c.deleted_at IS NULL
  AND NOT c.rekey
