WITH head AS (
    SELECT seq, created_at
    FROM chat.messages
    WHERE conversation_id = $1
    ORDER BY seq
    LIMIT $3
),
old AS (
    SELECT seq
    FROM head
    WHERE created_at < $2
),
gone AS (
    DELETE FROM chat.messages AS message
    USING old
    WHERE message.conversation_id = $1
      AND message.seq = old.seq
    RETURNING message.seq
),
refs AS (
    DELETE FROM chat.media_refs AS ref
    USING old
    WHERE ref.conversation_id = $1
      AND ref.seq = old.seq
    RETURNING ref.media_id, ref.seq
),
media AS (
    SELECT seq, array_agg(media_id) AS ids
    FROM refs
    GROUP BY seq
)
SELECT gone.seq AS "seq!", media.ids AS "media?"
FROM gone
LEFT JOIN media ON media.seq = gone.seq
