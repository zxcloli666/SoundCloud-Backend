SELECT id, provider, created_at, last_used_at
FROM remote_grants
WHERE sc_user_id = $1
  AND (external_id IS NOT NULL OR (access_hash IS NOT NULL AND refresh_expires_at > now()))
ORDER BY created_at DESC
