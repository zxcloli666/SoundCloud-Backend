UPDATE oauth_app_tokens
SET expires_at = LEAST(expires_at, now())
WHERE oauth_app_id = $1
  AND generation = $2
RETURNING oauth_app_id
