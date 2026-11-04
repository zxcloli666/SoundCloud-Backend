UPDATE chat.conversations
SET key_epoch = $2,
    rekey = false,
    epoch_messages = 0,
    epoch_started_at = now()
WHERE id = $1
