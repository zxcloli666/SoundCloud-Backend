DELETE
FROM sessions
WHERE id = $1
RETURNING soundcloud_connection_id
