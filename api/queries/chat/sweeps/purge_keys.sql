DELETE FROM chat.conversation_keys
WHERE conversation_id = $1
  AND EXISTS (
      SELECT 1
      FROM chat.conversations
      WHERE id = $1
        AND deleted_at IS NOT NULL
  )
