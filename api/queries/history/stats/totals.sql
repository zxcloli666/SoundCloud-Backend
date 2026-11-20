SELECT COUNT(*) FILTER (WHERE (listened.ms IS NULL OR listened.ms >= 30000)) AS "plays!",
       COALESCE(SUM(COALESCE(listened.ms, duration)::bigint), 0)::bigint               AS "listened_ms!",
       COUNT(DISTINCT CASE
                          WHEN sc_track_id ~ '^[0-9]+$' THEN 'soundcloud:tracks:' || sc_track_id
                          ELSE sc_track_id
           END) FILTER (WHERE (listened.ms IS NULL OR listened.ms >= 30000)) AS "tracks!",
       COUNT(DISTINCT COALESCE(artist_urn, lower(artist_name))) FILTER (WHERE (listened.ms IS NULL OR listened.ms >= 30000)) AS "artists!"
FROM listening_history
         LEFT JOIN LATERAL (SELECT MAX(listened_ms) AS ms
                            FROM listening_history_listened
                            WHERE history_id = listening_history.id) AS listened ON TRUE
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
