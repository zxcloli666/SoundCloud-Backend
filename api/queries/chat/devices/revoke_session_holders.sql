UPDATE chat.devices
SET status = 'revoked',
    revoked_at = now()
WHERE session_id = $1
  AND status = 'active'
  AND id <> $2
RETURNING id
