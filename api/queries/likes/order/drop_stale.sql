WITH live AS (
    SELECT DISTINCT ON (sc_track_id) sc_track_id, created_at, wanted_state
    FROM user_likes_tracks
    WHERE user_id = ANY($1)
    ORDER BY sc_track_id, (user_id = $2) DESC
)
DELETE FROM user_like_ranks ranked
WHERE ranked.user_id = $2
  AND NOT EXISTS (
      SELECT 1 FROM live
      WHERE live.sc_track_id = ranked.sc_track_id
        AND live.created_at = ranked.like_created_at
        AND live.wanted_state
  )
