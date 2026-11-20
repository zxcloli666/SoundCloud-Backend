SELECT EXTRACT(ISODOW FROM played_at + make_interval(mins => $4))::int AS "weekday!",
       EXTRACT(HOUR FROM played_at + make_interval(mins => $4))::int   AS "hour!",
       COUNT(*)                                                        AS "plays!"
FROM listening_history
         LEFT JOIN LATERAL (SELECT MAX(listened_ms) AS ms
                            FROM listening_history_listened
                            WHERE history_id = listening_history.id) AS listened ON TRUE
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
  AND (listened.ms IS NULL OR listened.ms >= 30000)
GROUP BY 1, 2
