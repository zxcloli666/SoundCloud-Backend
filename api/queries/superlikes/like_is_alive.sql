SELECT EXISTS(SELECT 1
              FROM user_likes_tracks
              WHERE user_id = ANY ($1)
                AND sc_track_id = $2
                AND wanted_state = true) AS "exists!"
