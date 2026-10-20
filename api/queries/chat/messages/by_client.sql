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
  AND client_id = $2
