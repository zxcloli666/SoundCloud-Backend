SELECT id, provider, created_at, last_used_at
FROM remote_grants
WHERE sc_user_id = $1
  AND (access_hash IS NOT NULL OR external_id IS NOT NULL)
ORDER BY created_at DESC
