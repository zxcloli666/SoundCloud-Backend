SELECT i.conversation_id, i.created_by, i.uses_left
FROM chat.invites i
JOIN chat.conversations c ON c.id = i.conversation_id
WHERE i.token_hash = $1
  AND i.revoked_at IS NULL
  AND i.expires_at > now()
  AND c.kind = 'group'
  AND c.deleted_at IS NULL
