SELECT count(*) AS "count!"
FROM chat.members m
JOIN chat.conversations c ON c.id = m.conversation_id AND c.deleted_at IS NULL
WHERE m.user_id = $1
  AND m.state = 'invited'
