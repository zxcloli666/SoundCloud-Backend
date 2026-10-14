WITH newest_ranked AS (
    SELECT max(found.created_at) AS created_at
    FROM unnest($1::text[]) AS variant(user_id)
    CROSS JOIN LATERAL (
        SELECT likes.created_at
        FROM user_likes_tracks likes
        JOIN user_like_ranks ranked
            ON ranked.user_id = $2
            AND ranked.sc_track_id = likes.sc_track_id
            AND ranked.like_created_at = likes.created_at
        WHERE likes.user_id = variant.user_id
          AND likes.wanted_state
          AND NOT $4
        ORDER BY likes.created_at DESC
        LIMIT 1
    ) found
),
since AS (
    SELECT COALESCE(created_at - interval '1 day', '-infinity'::timestamptz) AS created_at
    FROM newest_ranked
),
live AS (
    SELECT likes.sc_track_id, likes.created_at
    FROM user_likes_tracks likes
    CROSS JOIN since
    WHERE likes.user_id = ANY($1)
      AND likes.created_at > since.created_at
      AND likes.wanted_state
      AND (
          likes.user_id = $2
          OR NOT EXISTS (
              SELECT 1 FROM user_likes_tracks preferred
              WHERE preferred.user_id = $2 AND preferred.sc_track_id = likes.sc_track_id
          )
      )
),
unranked AS (
    SELECT live.sc_track_id,
           live.created_at,
           row_number() OVER (ORDER BY live.created_at DESC, live.sc_track_id DESC) AS position,
           count(*) OVER () AS total
    FROM live
    WHERE (
        SELECT ranked.like_created_at FROM user_like_ranks ranked
        WHERE ranked.user_id = $2 AND ranked.sc_track_id = live.sc_track_id
    ) IS DISTINCT FROM live.created_at
),
top AS (
    SELECT COALESCE(min(rank), 0) AS rank FROM user_like_ranks WHERE user_id = $2
)
INSERT INTO user_like_ranks (user_id, sc_track_id, like_created_at, rank)
SELECT $2, unranked.sc_track_id, unranked.created_at, top.rank - (unranked.total - unranked.position + 1) * $3
FROM unranked CROSS JOIN top
ON CONFLICT (user_id, sc_track_id) DO UPDATE
SET like_created_at = EXCLUDED.like_created_at, rank = EXCLUDED.rank
