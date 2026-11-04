UPDATE chat.conversations
SET deleted_at = now(),
    dm_key = NULL
WHERE id = $1
  AND deleted_at IS NULL
RETURNING last_seq
