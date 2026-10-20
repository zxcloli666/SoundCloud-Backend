SELECT key_epoch
FROM chat.conversations
WHERE id = $1
  AND deleted_at IS NULL
