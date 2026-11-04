DELETE FROM chat.messages
WHERE conversation_id = $1
  AND seq IN (
      SELECT seq
      FROM chat.messages
      WHERE conversation_id = $1
      ORDER BY seq
      LIMIT $2
      FOR UPDATE SKIP LOCKED
  )
  AND EXISTS (
      SELECT 1
      FROM chat.conversations
      WHERE id = $1
        AND deleted_at IS NOT NULL
  )
