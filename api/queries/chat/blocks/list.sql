SELECT b.blocked_user, b.created_at, u.username AS "username?", u.avatar_url AS "avatar_url?"
FROM chat.blocks b
LEFT JOIN users u ON u.sc_user_id = b.blocked_user
WHERE b.user_id = $1
ORDER BY b.created_at DESC
LIMIT $2
