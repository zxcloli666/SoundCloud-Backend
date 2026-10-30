UPDATE sessions
SET soundcloud_connection_id = $2,
    updated_at = now()
WHERE soundcloud_connection_id = $1
