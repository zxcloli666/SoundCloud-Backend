SELECT username, avatar_url, permalink_url, verified
FROM users
WHERE sc_user_id = $1
