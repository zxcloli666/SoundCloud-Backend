SELECT EXTRACT(ISODOW FROM played_at + make_interval(mins => $4))::int AS "weekday!",
       EXTRACT(HOUR FROM played_at + make_interval(mins => $4))::int   AS "hour!",
       COUNT(*)                                                        AS "plays!"
FROM listening_history
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
GROUP BY 1, 2
