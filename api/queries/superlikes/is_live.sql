SELECT EXISTS(SELECT 1
              FROM user_superlikes s
              JOIN user_likes_tracks l
                ON l.user_id = s.user_id
               AND l.sc_track_id = s.sc_track_id
               AND l.created_at = s.like_created_at
              WHERE s.user_id = ANY ($1)
                AND s.sc_track_id = $2
                AND l.wanted_state = true) AS "exists!"
