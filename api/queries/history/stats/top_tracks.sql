SELECT CASE
           WHEN sc_track_id ~ '^[0-9]+$' THEN 'soundcloud:tracks:' || sc_track_id
           ELSE sc_track_id
           END                                                                          AS "sc_track_id!",
       (array_agg(title ORDER BY played_at DESC))[1]                                        AS "title!",
       (array_agg(artist_name ORDER BY played_at DESC))[1]                                  AS "artist_name!",
       (array_agg(artist_urn ORDER BY played_at DESC) FILTER (WHERE artist_urn IS NOT NULL))[1] AS "artist_urn?",
       (array_agg(artwork_url ORDER BY played_at DESC) FILTER (WHERE artwork_url IS NOT NULL))[1] AS "artwork_url?",
       MAX(duration)                                                                        AS "duration!",
       COUNT(*)                                                                             AS "plays!",
       COALESCE(SUM(duration::bigint), 0)::bigint                                           AS "listened_ms!"
FROM listening_history
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
GROUP BY 1
ORDER BY COUNT(*) DESC, MAX(played_at) DESC
LIMIT $4
