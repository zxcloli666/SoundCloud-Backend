DELETE FROM user_superlike_ranks r
WHERE r.user_id = $2
  AND NOT EXISTS (
      SELECT 1
      FROM user_superlikes s
      JOIN user_likes_tracks l
        ON l.user_id = s.user_id
       AND l.sc_track_id = s.sc_track_id
       AND l.created_at = s.like_created_at
      WHERE s.user_id = ANY ($1)
        AND s.sc_track_id = r.sc_track_id
        AND s.created_at = r.superlike_created_at
        AND l.wanted_state = true
  )
