SELECT s.sc_track_id, s.created_at
FROM user_superlikes s
JOIN user_likes_tracks l
  ON l.user_id = s.user_id
 AND l.sc_track_id = s.sc_track_id
 AND l.created_at = s.like_created_at
JOIN tracks t
  ON t.sc_track_id = s.sc_track_id
 AND t.deleted_at IS NULL
WHERE s.user_id = ANY ($1)
  AND l.wanted_state = true
ORDER BY s.created_at DESC, s.sc_track_id DESC
LIMIT $2
