SELECT id, soundcloud_connection_id, chat_bind
FROM sessions
WHERE id = $1
FOR UPDATE
