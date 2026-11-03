UPDATE remote_grants
SET code_hash = NULL,
    code_redirect_uri = NULL,
    code_expires_at = NULL,
    access_hash = $2,
    access_expires_at = now() + ($3::int * INTERVAL '1 second'),
    refresh_hash = $4,
    refresh_expires_at = now() + ($5::int * INTERVAL '1 second'),
    last_used_at = now()
WHERE code_hash = $1
  AND client_id = $6
  AND code_redirect_uri = $7
  AND code_expires_at > now()
RETURNING id
