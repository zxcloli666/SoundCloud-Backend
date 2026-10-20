SELECT conversation_id,
       seq,
       client_id,
       sender_user,
       sender_device,
       class,
       epoch,
       target_seq,
       body,
       created_at,
       expires_at
FROM chat.messages
WHERE conversation_id = $1
  AND seq > $2
  AND (expires_at IS NULL OR expires_at > now())
ORDER BY seq
LIMIT $3
