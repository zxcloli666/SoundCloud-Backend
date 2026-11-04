INSERT INTO chat.messages (conversation_id, seq, client_id, sender_user, class, epoch, body)
VALUES ($1, $2, $3, $4, 'system', $5, $6)
RETURNING created_at
