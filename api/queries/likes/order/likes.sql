SELECT live.sc_track_id AS "sc_track_id!", live.created_at AS "created_at!"
FROM (
    SELECT DISTINCT ON (sc_track_id) sc_track_id, created_at, wanted_state
    FROM user_likes_tracks
    WHERE user_id = ANY($1) AND sc_track_id = ANY($3)
    ORDER BY sc_track_id, (user_id = $2) DESC
) live
WHERE live.wanted_state
