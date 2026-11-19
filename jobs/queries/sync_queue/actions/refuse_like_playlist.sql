UPDATE user_likes_playlists
SET wanted_state = false
WHERE user_id = $1
  AND playlist_urn = $2
  AND wanted_state = true
