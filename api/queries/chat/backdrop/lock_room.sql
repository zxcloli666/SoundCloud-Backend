SELECT id
FROM chat.conversations
WHERE id = $1
FOR NO KEY UPDATE
