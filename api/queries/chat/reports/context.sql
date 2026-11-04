SELECT seq, client_id, sender_user, sender_device, class, epoch, target_seq, body AS "body!", created_at
FROM chat.messages
WHERE conversation_id = $1
  AND seq < $2
  AND seq > $3
  AND body IS NOT NULL
ORDER BY seq DESC
LIMIT $4
