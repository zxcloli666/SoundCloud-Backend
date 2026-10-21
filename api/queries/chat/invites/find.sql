SELECT i.conversation_id, i.created_by, i.uses_left
FROM chat.invites i
JOIN chat.conversations c ON c.id = i.conversation_id
JOIN chat.members creator ON creator.conversation_id = i.conversation_id AND creator.user_id = i.created_by
WHERE i.token_hash = $1
  AND i.revoked_at IS NULL
  AND i.expires_at > now()
  AND c.kind = 'group'
  AND c.deleted_at IS NULL
  AND creator.state = 'active'
  AND creator.role IN ('owner', 'admin')
