SELECT e.sc_track_id,
       e.weight * CASE
                      WHEN e.event_type = 'like' AND EXISTS(SELECT 1
                                                            FROM user_superlikes s
                                                            JOIN user_likes_tracks l
                                                              ON l.user_id = s.user_id
                                                             AND l.sc_track_id = s.sc_track_id
                                                             AND l.created_at = s.like_created_at
                                                            WHERE s.user_id = ANY ($1)
                                                              AND s.sc_track_id = e.sc_track_id
                                                              AND l.wanted_state = true)
                          THEN $3::float8
                      ELSE 1.0
                  END AS "weight!",
       (EXTRACT(EPOCH FROM (NOW() - e.created_at)) / 86400.0) ::real AS "age_days!"
FROM user_events e
WHERE e.sc_user_id = ANY ($1)
  AND e.event_type IN ('like', 'playlist_add')
  AND e.created_at > NOW() - INTERVAL '365 days'
ORDER BY e.created_at DESC
    LIMIT $2
