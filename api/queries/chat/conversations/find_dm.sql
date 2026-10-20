SELECT id
FROM chat.conversations
WHERE dm_key = $1
  AND deleted_at IS NULL
