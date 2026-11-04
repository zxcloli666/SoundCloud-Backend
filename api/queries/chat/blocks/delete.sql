DELETE FROM chat.blocks
WHERE user_id = $1
  AND blocked_user = $2
