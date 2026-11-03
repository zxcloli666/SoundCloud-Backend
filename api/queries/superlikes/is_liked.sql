SELECT EXISTS(SELECT 1
              FROM user_likes_tracks l
              WHERE l.user_id = ANY ($1)
                AND l.sc_track_id = $2
                AND l.wanted_state = true) AS "exists!"
