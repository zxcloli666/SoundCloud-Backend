INSERT INTO remote_grants (id, sc_user_id, provider, external_id, last_used_at)
VALUES ($1, $2, 'salut', $3, now())
RETURNING id
