SELECT sc_track_id AS id, (EXTRACT(EPOCH FROM created_at) * 1000000)::int8 AS "at!"
FROM user_likes_tracks
WHERE user_id = ANY ($1)
  AND wanted_state = true
  AND created_at > NOW() - INTERVAL '365 days'
ORDER BY created_at DESC, ctid DESC
    LIMIT $2
