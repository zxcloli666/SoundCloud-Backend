UPDATE chat.conversations
SET deleted_at = now()
WHERE id = $1
  AND kind = 'room'
  AND deleted_at IS NULL
