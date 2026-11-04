UPDATE chat.messages
SET body = NULL
WHERE conversation_id = $1
  AND seq >= $2
  AND (seq = $2 OR (target_seq = $2 AND class = 'edit'))
  AND body IS NOT NULL
RETURNING conversation_id, seq, client_id, sender_user, sender_device, class, epoch, target_seq, body, created_at, expires_at
