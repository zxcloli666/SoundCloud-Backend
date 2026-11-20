INSERT INTO listening_history_listened (history_id, soundcloud_user_id, listened_ms)
SELECT id, soundcloud_user_id, $3
FROM listening_history
WHERE id = $1
  AND soundcloud_user_id = ANY ($2)
