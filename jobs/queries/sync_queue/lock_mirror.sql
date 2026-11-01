SELECT (
    (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM user_likes_tracks
            WHERE $1 IN ('like_track', 'unlike_track')
              AND user_id = regexp_replace($2, '^.*:', '')
              AND sc_track_id = regexp_replace($3, '^.*:', '')
            FOR UPDATE
        ) AS track
    ) + (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM user_likes_playlists
            WHERE $1 IN ('like_playlist', 'unlike_playlist')
              AND user_id = regexp_replace($2, '^.*:', '')
              AND playlist_urn = $3
            FOR UPDATE
        ) AS playlist
    ) + (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM user_followings
            WHERE $1 IN ('follow_user', 'unfollow_user')
              AND user_id = regexp_replace($2, '^.*:', '')
              AND target_user_urn = $3
            FOR UPDATE
        ) AS following
    )
) AS "locked!"
