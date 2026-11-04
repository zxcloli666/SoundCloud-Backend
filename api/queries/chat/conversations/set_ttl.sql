UPDATE chat.conversations
SET ttl_s = $2
WHERE id = $1
  AND deleted_at IS NULL
  AND ttl_s IS DISTINCT FROM $2
RETURNING id
