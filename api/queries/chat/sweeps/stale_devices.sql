SELECT id,
       session_id,
       last_seen_at
FROM chat.devices
WHERE status = 'active'
  AND last_seen_at < now() - interval '1 hour'
  AND user_id NOT LIKE 'lt:%'
  AND ($1::timestamptz IS NULL OR (last_seen_at, id) > ($1, $2::uuid))
ORDER BY last_seen_at, id
LIMIT $3
FOR UPDATE SKIP LOCKED
