UPDATE sessions
SET soundcloud_connection_id = $2,
    updated_at = now()
WHERE id = $1
RETURNING id
