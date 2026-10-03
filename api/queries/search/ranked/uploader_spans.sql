SELECT sc_user_id,
       username_normalized
FROM users
WHERE username_normalized = ANY ($1)
ORDER BY followers_count DESC NULLS LAST
LIMIT 8
