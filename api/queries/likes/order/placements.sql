SELECT live.sc_track_id AS "sc_track_id!", live.created_at AS "created_at!", ranked.rank AS "rank?"
FROM (
    SELECT DISTINCT ON (sc_track_id) sc_track_id, created_at, wanted_state
    FROM user_likes_tracks
    WHERE user_id = ANY($1) AND sc_track_id = ANY($3)
    ORDER BY sc_track_id, (user_id = $2) DESC
) live
LEFT JOIN user_like_ranks ranked
    ON ranked.user_id = $2
    AND ranked.sc_track_id = live.sc_track_id
    AND ranked.like_created_at = live.created_at
WHERE live.wanted_state
