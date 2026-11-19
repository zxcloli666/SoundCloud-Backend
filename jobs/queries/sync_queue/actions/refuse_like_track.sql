UPDATE user_likes_tracks
SET wanted_state = false
WHERE user_id = $1
  AND sc_track_id = $2
  AND wanted_state = true
