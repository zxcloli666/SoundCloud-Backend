DELETE FROM remote_grants
WHERE id = $1
RETURNING id
