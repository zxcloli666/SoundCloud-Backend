SELECT session.chat_bind,
       connection.soundcloud_user_id AS "soundcloud_user_id?"
FROM sessions AS session
LEFT JOIN soundcloud_connections AS connection
    ON connection.id = session.soundcloud_connection_id
WHERE session.id = $1
