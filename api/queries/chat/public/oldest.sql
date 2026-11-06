SELECT created_at
FROM chat.messages
WHERE conversation_id = $1
ORDER BY seq
LIMIT 1
