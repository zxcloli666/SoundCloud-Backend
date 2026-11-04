SELECT min(deleted_at) AS oldest
FROM chat.conversations
WHERE deleted_at IS NOT NULL
