WITH expired AS (
    SELECT conversation_id, seq
    FROM chat.messages
    WHERE expires_at <= now()
    ORDER BY expires_at
    LIMIT $1
    FOR UPDATE SKIP LOCKED
),
gone AS (
    DELETE FROM chat.messages AS message
    USING expired
    WHERE message.conversation_id = expired.conversation_id
      AND message.seq = expired.seq
    RETURNING message.conversation_id, message.seq
),
refs AS (
    DELETE FROM chat.media_refs AS ref
    USING expired
    WHERE ref.conversation_id = expired.conversation_id
      AND ref.seq = expired.seq
    RETURNING ref.media_id, ref.conversation_id, ref.seq
),
media AS (
    SELECT conversation_id, seq, array_agg(media_id) AS ids
    FROM refs
    GROUP BY conversation_id, seq
)
SELECT gone.conversation_id AS "conversation_id!", gone.seq AS "seq!", media.ids AS "media?"
FROM gone
LEFT JOIN media
  ON media.conversation_id = gone.conversation_id
 AND media.seq = gone.seq
