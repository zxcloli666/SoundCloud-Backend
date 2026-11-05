UPDATE chat.devices
SET status = 'revoked',
    revoked_at = now()
WHERE session_hash = $1
  AND status = 'active'
  AND id <> $2
RETURNING id
