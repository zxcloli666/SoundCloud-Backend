UPDATE chat.invites i
SET revoked_at = now()
WHERE i.token_hash = $1
  AND i.revoked_at IS NULL
  AND (
      i.created_by = $2
      OR EXISTS (
          SELECT 1
          FROM chat.members m
          WHERE m.conversation_id = i.conversation_id
            AND m.user_id = $2
            AND m.state = 'active'
            AND m.role IN ('owner', 'admin')
      )
  )
RETURNING i.conversation_id
