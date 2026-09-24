WITH listens AS (SELECT sc_track_id,
                        MAX(created_at)                   AS last_listen,
                        bool_or(event_type = 'full_play') AS fully
                 FROM user_events
                 WHERE sc_user_id = ANY ($1)
                   AND event_type IN ('full_play', 'skip')
                   AND created_at > NOW() - INTERVAL '30 days'
                 GROUP BY sc_track_id),
     liked AS (SELECT DISTINCT sc_track_id
               FROM user_likes_tracks
               WHERE user_id = ANY ($1)
                 AND wanted_state = true)
SELECT ls.sc_track_id AS "sc_track_id!"
FROM listens ls
         LEFT JOIN liked l USING (sc_track_id)
WHERE CASE
          WHEN l.sc_track_id IS NOT NULL THEN ls.last_listen > NOW() - INTERVAL '7 days'
          WHEN ls.fully THEN ls.last_listen > NOW() - INTERVAL '14 days'
          ELSE ls.last_listen > NOW() - INTERVAL '30 days'
          END
