UPDATE chat.conversations
SET meta_ct = $2,
    meta_epoch = $3
WHERE id = $1
  AND deleted_at IS NULL
RETURNING id
