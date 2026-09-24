SELECT sc_user_id, sc_track_id, created_at
FROM user_events
WHERE created_at >= $1
  AND event_type = ANY ($2)
ORDER BY max(created_at) OVER (PARTITION BY sc_user_id) DESC, sc_user_id, created_at
