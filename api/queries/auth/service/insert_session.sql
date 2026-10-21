INSERT INTO sessions (id, soundcloud_connection_id, chat_bind)
VALUES ($1, $2, $3)
RETURNING id
