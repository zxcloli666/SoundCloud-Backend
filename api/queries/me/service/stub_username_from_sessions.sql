SELECT connection.username AS "username!"
FROM sessions AS session
JOIN soundcloud_connections AS connection
    ON connection.id = session.soundcloud_connection_id
WHERE connection.soundcloud_user_id = ANY ($1)
  AND connection.username IS NOT NULL
ORDER BY session.updated_at DESC
LIMIT 1
