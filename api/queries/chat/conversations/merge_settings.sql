UPDATE chat.conversations
SET settings = settings || $2
WHERE id = $1
  AND deleted_at IS NULL
RETURNING settings
