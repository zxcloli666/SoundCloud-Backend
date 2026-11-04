SELECT c.id,
       c.kind,
       COALESCE(m.invited_by, c.created_by) AS "from_user!",
       m.updated_at,
       c.last_seq,
       c.last_at,
       c.member_count,
       c.meta_ct,
       c.meta_epoch
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
ORDER BY c.last_at DESC
LIMIT $2
