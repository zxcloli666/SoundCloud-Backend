INSERT INTO chat.media_refs (media_id, conversation_id, seq)
VALUES ($1, $2, 0)
ON CONFLICT DO NOTHING
