INSERT INTO chat.blocks (user_id, blocked_user)
SELECT $1, $2
WHERE (SELECT count(*) FROM chat.blocks WHERE user_id = $1) < $3
ON CONFLICT (user_id, blocked_user) DO NOTHING
RETURNING blocked_user
