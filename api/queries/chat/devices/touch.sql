UPDATE chat.devices
SET last_seen_at = now()
WHERE id = $1
  AND status = 'active'
