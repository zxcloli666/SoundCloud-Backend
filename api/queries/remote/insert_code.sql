INSERT INTO remote_link_codes (id, sc_user_id, provider, code_hash, expires_at)
VALUES ($1, $2, $3, $4, now() + ($5::int * INTERVAL '1 second'))
RETURNING expires_at
