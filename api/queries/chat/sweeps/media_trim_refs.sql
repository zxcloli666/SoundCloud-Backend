DELETE FROM chat.media_refs AS ref
USING (
    SELECT media_id, conversation_id, seq
    FROM chat.media_refs
    WHERE conversation_id = $1
      AND seq > 0
      AND seq <= $2
    ORDER BY seq
    LIMIT $3
    FOR UPDATE
) AS due
WHERE ref.media_id = due.media_id
  AND ref.conversation_id = due.conversation_id
  AND ref.seq = due.seq
RETURNING ref.media_id
