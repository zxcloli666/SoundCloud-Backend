DELETE FROM chat.messages AS message
USING (
    SELECT conversation_id, seq
    FROM chat.messages
    WHERE expires_at <= now()
    ORDER BY expires_at
    LIMIT $1
    FOR UPDATE SKIP LOCKED
) AS expired
WHERE message.conversation_id = expired.conversation_id
  AND message.seq = expired.seq
RETURNING message.conversation_id, message.seq
