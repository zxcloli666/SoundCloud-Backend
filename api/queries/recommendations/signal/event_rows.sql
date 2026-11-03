SELECT event.sc_track_id AS "sc_track_id!",
       event.event_type AS "event_type!",
       event.weight AS "weight!",
       event.position_pct,
       (EXTRACT(EPOCH FROM (NOW() - event.created_at)) / 86400.0) ::real AS "age_days!"
FROM (SELECT DISTINCT user_id FROM unnest($1::text[]) AS user_id) AS users
CROSS JOIN (SELECT DISTINCT event_type FROM unnest($2::text[]) AS event_type) AS types
CROSS JOIN LATERAL (
    SELECT id, sc_track_id, event_type, weight, position_pct, created_at
    FROM user_events
    WHERE sc_user_id = users.user_id
      AND event_type = types.event_type
      AND created_at > NOW() - INTERVAL '180 days'
    ORDER BY created_at DESC
    LIMIT $3
) AS event
ORDER BY event.created_at DESC, event.id DESC
LIMIT $3
