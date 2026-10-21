UPDATE sessions
SET soundcloud_connection_id = $2,
    chat_bind = COALESCE($3, chat_bind),
    updated_at = now()
WHERE id = $1
RETURNING id
