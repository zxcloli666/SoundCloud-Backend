SELECT count(*) AS "count!"
FROM chat.members m
JOIN chat.conversations c ON c.id = m.conversation_id
WHERE m.user_id = $1
  AND m.state IN ('active', 'invited')
  AND NOT m.accepted
  AND c.kind IN ('direct', 'group')
  AND c.deleted_at IS NULL
  AND NOT EXISTS (
      SELECT 1
      FROM chat.blocks b
      WHERE b.user_id = $1
        AND b.blocked_user = COALESCE(m.invited_by, c.created_by)
  )
