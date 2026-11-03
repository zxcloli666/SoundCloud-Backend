INSERT INTO remote_grants (id, sc_user_id, provider, client_id, code_hash, code_redirect_uri, code_expires_at)
VALUES ($1, $2, $3, $4, $5, $6, now() + ($7::int * INTERVAL '1 second'))
RETURNING id
