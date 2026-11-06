DELETE FROM chat.public_mutes
WHERE user_id = $1
RETURNING until
