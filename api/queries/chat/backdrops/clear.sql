UPDATE chat.conversations
SET settings = settings - 'backdrop'
WHERE id = $1
