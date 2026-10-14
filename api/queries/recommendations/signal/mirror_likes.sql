SELECT m.sc_track_id,
       (EXTRACT(EPOCH FROM (NOW() - m.created_at)) / 86400.0) ::real AS "age_days!",
       CASE
           WHEN EXISTS(SELECT 1
                       FROM user_superlikes s
                       WHERE s.user_id = m.user_id
                         AND s.sc_track_id = m.sc_track_id
                         AND s.like_created_at = m.created_at)
               THEN $3::real
           ELSE 1.0::real
       END AS "boost!"
FROM user_likes_tracks m
WHERE m.user_id = ANY ($1)
  AND m.wanted_state = true
  AND m.created_at > NOW() - INTERVAL '365 days'
ORDER BY m.created_at DESC, m.ctid DESC
    LIMIT $2
