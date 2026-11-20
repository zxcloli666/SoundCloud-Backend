SELECT date_trunc($4, played_at + make_interval(mins => $5)) AS "bucket!",
       COUNT(*) FILTER (WHERE (listened.ms IS NULL OR listened.ms >= 30000)) AS "plays!",
       COALESCE(SUM(COALESCE(listened.ms, duration)::bigint), 0)::bigint            AS "listened_ms!"
FROM listening_history
         LEFT JOIN LATERAL (SELECT MAX(listened_ms) AS ms
                            FROM listening_history_listened
                            WHERE history_id = listening_history.id) AS listened ON TRUE
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
GROUP BY 1
ORDER BY 1
