UPDATE chat.conversations
SET last_seq = last_seq + $2,
    last_at = now(),
    rekey = rekey OR $3
WHERE id = $1
  AND kind = 'room'
  AND deleted_at IS NULL
RETURNING last_seq, key_epoch
