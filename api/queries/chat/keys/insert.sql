INSERT INTO chat.conversation_keys (conversation_id, epoch, wrapped, kek_version, created_at)
VALUES ($1, $2, $3, $4, clock_timestamp())
ON CONFLICT (conversation_id, epoch) DO NOTHING
