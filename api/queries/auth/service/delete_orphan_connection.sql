DELETE FROM soundcloud_connections AS connection
WHERE connection.id = $1
  AND NOT EXISTS (
      SELECT 1
      FROM sessions AS session
      WHERE session.soundcloud_connection_id = connection.id
  )
