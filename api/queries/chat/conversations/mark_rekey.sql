UPDATE chat.conversations
SET rekey = true
WHERE id = $1
  AND deleted_at IS NULL
  AND NOT rekey
