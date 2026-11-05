DELETE FROM chat.media_refs AS ref
USING (
    SELECT media_id, conversation_id, seq
    FROM chat.media_refs
    WHERE conversation_id = $1
    ORDER BY seq
    LIMIT $2
    FOR UPDATE
) AS due
WHERE ref.media_id = due.media_id
  AND ref.conversation_id = due.conversation_id
  AND ref.seq = due.seq
  AND EXISTS (
      SELECT 1
      FROM chat.conversations
      WHERE id = $1
        AND deleted_at IS NOT NULL
  )
RETURNING ref.media_id
