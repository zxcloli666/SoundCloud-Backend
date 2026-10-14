SELECT m.sc_track_id AS "sc_track_id!"
FROM (
    SELECT live.sc_track_id, live.created_at, ranked.rank
    FROM (
        SELECT DISTINCT ON (sc_track_id) sc_track_id, created_at, wanted_state
        FROM user_likes_tracks
        WHERE user_id = ANY($1)
        ORDER BY sc_track_id, (user_id = $4) DESC
    ) live
    LEFT JOIN user_like_ranks ranked
        ON ranked.user_id = $4
        AND ranked.sc_track_id = live.sc_track_id
        AND ranked.like_created_at = live.created_at
    WHERE live.wanted_state
    ORDER BY ranked.rank NULLS FIRST, live.created_at DESC, live.sc_track_id DESC
    OFFSET 0
) m
CROSS JOIN LATERAL (
    SELECT 1 AS present FROM tracks e
    WHERE e.sc_track_id = m.sc_track_id AND e.deleted_at IS NULL
    OFFSET 0
) e
ORDER BY m.rank NULLS FIRST, m.created_at DESC, m.sc_track_id DESC
LIMIT $2 OFFSET $3
