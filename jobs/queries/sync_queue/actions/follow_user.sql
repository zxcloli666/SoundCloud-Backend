UPDATE user_followings
SET progress = false,
    synced_at = now(),
    synced_for_v = wanted_v
WHERE user_id = $1
  AND target_user_urn = $2
  AND wanted_state = true
