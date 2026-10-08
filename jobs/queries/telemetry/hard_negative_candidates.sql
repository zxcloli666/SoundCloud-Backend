SELECT event.id AS "event_id!",
       event.sc_user_id AS "sc_user_id!",
       event.sc_track_id AS "sc_track_id!",
       event.position_pct AS "position_pct!",
       event.created_at AS "created_at!",
       (event.created_at AT TIME ZONE 'UTC') AS "detected_at!"
FROM user_events AS event
WHERE event.created_at >= $1
  AND (event.created_at > $1 OR event.id > $2)
  AND event.created_at <= (now() AT TIME ZONE 'UTC') - $3::bigint * interval '1 second'
  AND event.event_type = 'skip'
  AND event.position_pct >= 0
  AND event.position_pct < 0.2
ORDER BY event.created_at, event.id
LIMIT $4
