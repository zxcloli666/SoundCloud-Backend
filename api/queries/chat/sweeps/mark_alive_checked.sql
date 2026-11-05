UPDATE chat.devices
SET alive_checked_at = $2
WHERE id = ANY ($1)
  AND status = 'active'
