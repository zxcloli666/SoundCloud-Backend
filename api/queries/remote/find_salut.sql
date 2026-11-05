SELECT id, sc_user_id, provider
FROM remote_grants
WHERE provider = 'salut' AND external_id = $1
ORDER BY created_at DESC
LIMIT 1
