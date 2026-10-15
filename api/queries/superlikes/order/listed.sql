SELECT s.sc_track_id, s.created_at, r.rank AS "rank?"
FROM user_superlikes s
JOIN user_likes_tracks l
  ON l.user_id = s.user_id
 AND l.sc_track_id = s.sc_track_id
 AND l.created_at = s.like_created_at
JOIN tracks t
  ON t.sc_track_id = s.sc_track_id
 AND t.deleted_at IS NULL
LEFT JOIN user_superlike_ranks r
  ON r.user_id = $2
 AND r.sc_track_id = s.sc_track_id
 AND r.superlike_created_at = s.created_at
WHERE s.user_id = ANY ($1)
  AND l.wanted_state = true
ORDER BY r.rank NULLS FIRST, s.created_at DESC, s.sc_track_id DESC
