SELECT soundcloud_user_id
FROM soundcloud_connections
WHERE id = $1
FOR UPDATE
