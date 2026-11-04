UPDATE chat.devices
SET history_unlocked_at = now()
WHERE id = $1
  AND status = 'active'
  AND history_unlocked_at IS NULL
