UPDATE chat.conversations
SET key_epoch = key_epoch + 1,
    rekey = false,
    epoch_messages = 0,
    epoch_started_at = now()
WHERE id = $1
  AND key_epoch = $2
  AND deleted_at IS NULL
  AND (rekey OR epoch_messages >= $3 OR epoch_started_at <= now() - $4::bigint * interval '1 day')
RETURNING key_epoch
