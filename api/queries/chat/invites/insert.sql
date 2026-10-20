INSERT INTO chat.invites (token_hash, conversation_id, created_by, expires_at, uses_left)
VALUES ($1, $2, $3, $4, NULLIF($5::int, 0))
