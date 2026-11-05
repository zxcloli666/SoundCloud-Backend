SELECT id, sc_user_id, provider
FROM remote_grants
WHERE access_hash = $1 AND access_expires_at > now()
