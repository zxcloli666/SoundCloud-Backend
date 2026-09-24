INSERT INTO sessions (id, soundcloud_connection_id)
VALUES ($1, $2)
RETURNING id
