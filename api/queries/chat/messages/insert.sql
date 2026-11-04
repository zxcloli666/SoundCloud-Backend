INSERT INTO chat.messages (conversation_id, seq, client_id, sender_user, sender_device, class, epoch, target_seq, body,
                           created_at, expires_at)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
ON CONFLICT (conversation_id, client_id) DO NOTHING
RETURNING seq
