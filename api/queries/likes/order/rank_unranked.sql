WITH live AS (
    SELECT DISTINCT ON (sc_track_id) sc_track_id, created_at, wanted_state
    FROM user_likes_tracks
    WHERE user_id = ANY($1)
    ORDER BY sc_track_id, (user_id = $2) DESC
),
unranked AS (
    SELECT live.sc_track_id,
           live.created_at,
           row_number() OVER (ORDER BY live.created_at DESC, live.sc_track_id DESC) AS position,
           count(*) OVER () AS total
    FROM live
    WHERE live.wanted_state
      AND NOT EXISTS (
          SELECT 1 FROM user_like_ranks ranked
          WHERE ranked.user_id = $2
            AND ranked.sc_track_id = live.sc_track_id
            AND ranked.like_created_at = live.created_at
      )
),
top AS (
    SELECT COALESCE(min(rank), 0) AS rank FROM user_like_ranks WHERE user_id = $2
)
INSERT INTO user_like_ranks (user_id, sc_track_id, like_created_at, rank)
SELECT $2, unranked.sc_track_id, unranked.created_at, top.rank - (unranked.total - unranked.position + 1) * $3
FROM unranked CROSS JOIN top
ON CONFLICT (user_id, sc_track_id) DO UPDATE
SET like_created_at = EXCLUDED.like_created_at, rank = EXCLUDED.rank
