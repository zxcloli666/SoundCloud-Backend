UPDATE chat.devices
SET status = 'revoked',
    revoked_at = now()
WHERE id = $1
  AND status = 'active'
RETURNING id
