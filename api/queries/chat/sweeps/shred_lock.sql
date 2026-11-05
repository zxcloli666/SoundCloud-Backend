SELECT id
FROM chat.conversations
WHERE id = $1
FOR UPDATE
