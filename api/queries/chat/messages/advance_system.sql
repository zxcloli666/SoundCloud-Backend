UPDATE chat.conversations
SET last_seq = last_seq + 1,
    last_at = now()
WHERE id = $1
  AND deleted_at IS NULL
RETURNING last_seq, key_epoch, now() AS "at!"
