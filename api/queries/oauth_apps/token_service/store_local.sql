INSERT INTO oauth_app_tokens_local (oauth_app_id, generation, access_token, expires_at, scope, issued_at)
VALUES ($1, $2, $3, $4, NULLIF($5, ''), now())
ON CONFLICT (oauth_app_id) DO UPDATE
SET generation = EXCLUDED.generation,
    access_token = EXCLUDED.access_token,
    expires_at = EXCLUDED.expires_at,
    scope = EXCLUDED.scope,
    issued_at = EXCLUDED.issued_at
WHERE oauth_app_tokens_local.expires_at < EXCLUDED.expires_at
RETURNING oauth_app_id
