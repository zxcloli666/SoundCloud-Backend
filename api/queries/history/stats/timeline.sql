SELECT date_trunc($4, played_at + make_interval(mins => $5)) AS "bucket!",
       COUNT(*)                                              AS "plays!",
       COALESCE(SUM(duration::bigint), 0)::bigint            AS "listened_ms!"
FROM listening_history
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
GROUP BY 1
ORDER BY 1
