DELETE FROM chat.conversations
WHERE id = $1
  AND deleted_at IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM chat.messages
      WHERE conversation_id = $1
  )
