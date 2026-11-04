SELECT id
FROM chat.conversations
WHERE deleted_at IS NOT NULL
ORDER BY deleted_at
LIMIT $1
