DELETE FROM chat.members AS member
WHERE member.user_id LIKE 'lt:%'
  AND EXISTS (
      SELECT 1
      FROM chat.conversations AS conversation
      WHERE conversation.id = member.conversation_id
        AND conversation.deleted_at IS NULL
  )
RETURNING member.conversation_id
