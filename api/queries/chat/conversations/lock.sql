SELECT last_seq,
       key_epoch,
       settings
FROM chat.conversations
WHERE id = $1
  AND deleted_at IS NULL
FOR UPDATE
