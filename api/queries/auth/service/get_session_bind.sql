SELECT connection.soundcloud_user_id,
       session.chat_bind
FROM sessions AS session
JOIN soundcloud_connections AS connection
    ON connection.id = session.soundcloud_connection_id
WHERE session.id = $1
