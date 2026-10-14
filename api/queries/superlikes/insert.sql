INSERT INTO user_superlikes (user_id, sc_track_id, like_created_at)
SELECT l.user_id, l.sc_track_id, l.created_at
FROM user_likes_tracks l
WHERE l.user_id = ANY ($1)
  AND l.sc_track_id = $2
  AND l.wanted_state = true
ORDER BY l.created_at DESC
LIMIT 1
ON CONFLICT (user_id, sc_track_id) DO UPDATE
SET like_created_at = EXCLUDED.like_created_at,
    created_at = now()
