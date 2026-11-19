SELECT connection.username,
       catalog.avatar_url AS "avatar_url?"
FROM sessions AS session
JOIN soundcloud_connections AS connection
    ON connection.id = session.soundcloud_connection_id
LEFT JOIN users AS catalog
    ON catalog.sc_user_id = connection.soundcloud_user_id
WHERE session.id = $1
