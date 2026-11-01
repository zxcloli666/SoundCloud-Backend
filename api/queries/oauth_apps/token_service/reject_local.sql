UPDATE oauth_app_tokens_local
SET expires_at = now()
WHERE oauth_app_id = $1
  AND generation = $2
  AND expires_at > now()
RETURNING oauth_app_id
