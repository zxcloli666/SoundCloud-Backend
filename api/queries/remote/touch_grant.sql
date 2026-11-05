UPDATE remote_grants
SET last_used_at = now()
WHERE id = $1
  AND (last_used_at IS NULL OR last_used_at < now() - INTERVAL '5 minutes')
