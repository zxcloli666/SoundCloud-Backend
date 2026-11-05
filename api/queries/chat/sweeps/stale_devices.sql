SELECT id,
       session_hash
FROM chat.devices
WHERE status = 'active'
  AND last_seen_at < now() - interval '1 hour'
  AND user_id NOT LIKE 'lt:%'
  AND (alive_checked_at IS NULL OR alive_checked_at < $1)
ORDER BY alive_checked_at NULLS FIRST, id
LIMIT $2
FOR UPDATE SKIP LOCKED
