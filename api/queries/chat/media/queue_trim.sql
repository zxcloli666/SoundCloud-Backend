INSERT INTO chat.media_trim (conversation_id)
VALUES ($1)
ON CONFLICT (conversation_id) DO UPDATE
SET queued_at = now()
