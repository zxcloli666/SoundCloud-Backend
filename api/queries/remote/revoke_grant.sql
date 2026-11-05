DELETE FROM remote_grants
WHERE id = $1 AND sc_user_id = $2
RETURNING id
