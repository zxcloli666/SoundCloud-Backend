SELECT COALESCE(u.typing, true) AS "typing!"
FROM chat.conversations c
JOIN chat.members m ON m.conversation_id = c.id AND m.user_id = $2 AND m.state = 'active'
LEFT JOIN chat.users u ON u.user_id = m.user_id
WHERE c.id = $1
  AND c.deleted_at IS NULL
