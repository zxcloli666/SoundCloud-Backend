SELECT id, sc_user_id
FROM remote_link_codes
WHERE code_hash = $1 AND provider = $2 AND expires_at > now()
ORDER BY created_at
LIMIT 1
