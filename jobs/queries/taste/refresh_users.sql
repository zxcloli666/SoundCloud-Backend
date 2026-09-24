SELECT DISTINCT regexp_replace(sc_user_id, '^.*:', '') AS "user_id!"
FROM user_events
WHERE created_at > $1
  AND created_at <= $2
