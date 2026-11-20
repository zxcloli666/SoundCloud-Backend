SELECT CASE
           WHEN sc_track_id ~ '^[0-9]+$' THEN 'soundcloud:tracks:' || sc_track_id
           ELSE sc_track_id
           END                                                                          AS "sc_track_id!",
       (array_agg(title ORDER BY played_at DESC))[1]                                        AS "title!",
       (array_agg(artist_name ORDER BY played_at DESC))[1]                                  AS "artist_name!",
       (array_agg(artist_urn ORDER BY played_at DESC) FILTER (WHERE artist_urn IS NOT NULL))[1] AS "artist_urn?",
       (array_agg(artwork_url ORDER BY played_at DESC) FILTER (WHERE artwork_url IS NOT NULL))[1] AS "artwork_url?",
       MAX(duration)                                                                        AS "duration!",
       COUNT(*) FILTER (WHERE (listened.ms IS NULL OR listened.ms >= 30000)) AS "plays!",
       COALESCE(SUM(COALESCE(listened.ms, duration)::bigint), 0)::bigint                                           AS "listened_ms!"
FROM listening_history
         LEFT JOIN LATERAL (SELECT MAX(listened_ms) AS ms
                            FROM listening_history_listened
                            WHERE history_id = listening_history.id) AS listened ON TRUE
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
GROUP BY 1
HAVING COUNT(*) FILTER (WHERE (listened.ms IS NULL OR listened.ms >= 30000)) > 0
ORDER BY COUNT(*) FILTER (WHERE (listened.ms IS NULL OR listened.ms >= 30000)) DESC, MAX(played_at) DESC
LIMIT $4
