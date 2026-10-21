SELECT m.role, m.state, c.kind, (c.deleted_at IS NULL) AS "live!"
FROM chat.members m
JOIN chat.conversations c ON c.id = m.conversation_id
WHERE m.conversation_id = $1
  AND m.user_id = $2
