UPDATE user_followings
SET wanted_state = false
WHERE user_id = $1
  AND target_user_urn = $2
  AND wanted_state = true
