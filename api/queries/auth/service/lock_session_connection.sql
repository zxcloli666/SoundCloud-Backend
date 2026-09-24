SELECT id, soundcloud_connection_id
FROM sessions
WHERE id = $1
FOR UPDATE
