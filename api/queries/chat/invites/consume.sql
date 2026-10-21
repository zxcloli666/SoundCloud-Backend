UPDATE chat.invites
SET uses_left = uses_left - 1
WHERE token_hash = $1
  AND revoked_at IS NULL
  AND expires_at > now()
  AND (uses_left IS NULL OR uses_left > 0)
RETURNING conversation_id, created_by
